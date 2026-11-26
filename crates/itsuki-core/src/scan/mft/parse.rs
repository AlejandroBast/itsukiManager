//! Lectura de registros de la MFT de NTFS. Código puro (sin E/S): recibe
//! bytes y devuelve información de archivos, así se puede probar en cualquier
//! sistema con registros sintéticos.
//!
//! Formato de referencia: registros `FILE` de 1024 o 4096 bytes protegidos con
//! una "update sequence" cada 512 bytes, seguidos de atributos tipo-longitud.

use super::super::util::{filetime_to_unix, push_name};

pub(crate) const ATTR_STANDARD_INFORMATION: u32 = 0x10;
pub(crate) const ATTR_ATTRIBUTE_LIST: u32 = 0x20;
pub(crate) const ATTR_FILE_NAME: u32 = 0x30;
pub(crate) const ATTR_DATA: u32 = 0x80;
const ATTR_END: u32 = 0xFFFF_FFFF;

/// Bits de la cabecera del registro.
const RECORD_IN_USE: u16 = 0x0001;
const RECORD_IS_DIRECTORY: u16 = 0x0002;

/// Bits de las banderas de un atributo.
const ATTR_FLAG_COMPRESSED: u16 = 0x0001;
const ATTR_FLAG_SPARSE: u16 = 0x8000;

/// Espacio de nombres "solo DOS" (nombre corto 8.3 duplicado): se ignora.
const NAMESPACE_DOS: u8 = 2;

/// Las referencias de archivo usan 48 bits para el número de registro.
pub(crate) const FRN_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

/// Paso de la "update sequence" (siempre 512 bytes en NTFS).
const USA_STRIDE: usize = 512;

/// `RecInfo::flags`
pub(crate) const IN_USE: u8 = 1;
pub(crate) const IS_DIR: u8 = 2;

/// Lo que importa de un registro base.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RecInfo {
    /// Tamaño lógico del flujo de datos principal.
    pub size: u64,
    /// Espacio en disco de todos sus atributos no residentes (datos, flujos
    /// alternativos, índices de carpeta…).
    pub alloc: u64,
    /// Última modificación en segundos Unix.
    pub mtime: i64,
    /// Atributos de archivo de Windows ($STANDARD_INFORMATION).
    pub attrs: u32,
    /// Etiqueta de reparse (si es un punto de reparse).
    pub tag: u32,
    pub flags: u8,
}

/// Un nombre del archivo dentro de una carpeta (un archivo con enlaces duros
/// tiene varios).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Link {
    pub frn: u32,
    pub parent: u32,
    pub name_off: u32,
    pub name_len: u16,
}

/// Datos de un registro de extensión que pertenecen a su registro base.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ExtPart {
    pub base: u32,
    pub size: Option<u64>,
    pub alloc: u64,
    pub tag: u32,
}

/// Resultado de analizar un bloque de registros.
#[derive(Debug, Default)]
pub(crate) struct ParseOut {
    pub links: Vec<Link>,
    pub names: Vec<u8>,
    pub ext: Vec<ExtPart>,
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
}

impl ParseOut {
    /// Añade otro resultado (ajustando las posiciones de los nombres).
    pub fn append(&mut self, mut other: ParseOut) {
        let base = self.names.len() as u32;
        self.names.append(&mut other.names);
        self.links.extend(other.links.into_iter().map(|l| Link { name_off: l.name_off + base, ..l }));
        self.ext.append(&mut other.ext);
        self.files += other.files;
        self.dirs += other.dirs;
        self.bytes += other.bytes;
    }
}

pub(crate) fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

pub(crate) fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

pub(crate) fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

/// Deshace la "update sequence" de un registro: los dos últimos bytes de
/// cada bloque de 512 se guardaron aparte y en su lugar hay un número de
/// control. Devuelve `false` si el registro está dañado o incompleto.
pub(crate) fn apply_fixup(rec: &mut [u8]) -> bool {
    let (Some(usa_off), Some(count)) = (u16_at(rec, 4), u16_at(rec, 6)) else { return false };
    let (usa_off, count) = (usa_off as usize, count as usize);
    if count < 2 || usa_off + count * 2 > rec.len() || (count - 1) * USA_STRIDE > rec.len() {
        return false;
    }
    let check = [rec[usa_off], rec[usa_off + 1]];
    for i in 1..count {
        let p = i * USA_STRIDE - 2;
        if rec[p..p + 2] != check {
            return false;
        }
        rec[p] = rec[usa_off + i * 2];
        rec[p + 1] = rec[usa_off + i * 2 + 1];
    }
    true
}

/// Atributo dentro de un registro.
pub(crate) struct Attr<'a> {
    pub ty: u32,
    pub non_resident: bool,
    pub name_len: u8,
    pub flags: u16,
    /// Bytes completos del atributo (cabecera incluida).
    pub bytes: &'a [u8],
}

impl<'a> Attr<'a> {
    /// Valor de un atributo residente.
    pub fn value(&self) -> Option<&'a [u8]> {
        if self.non_resident {
            return None;
        }
        let len = u32_at(self.bytes, 0x10)? as usize;
        let off = u16_at(self.bytes, 0x14)? as usize;
        self.bytes.get(off..off.checked_add(len)?)
    }

    /// VCN inicial de un atributo no residente (0 en el primer tramo).
    pub fn lowest_vcn(&self) -> Option<u64> {
        if self.non_resident { u64_at(self.bytes, 0x10) } else { None }
    }

    /// Tamaño lógico de un atributo no residente (válido si `lowest_vcn == 0`).
    pub fn data_size(&self) -> Option<u64> {
        if self.non_resident { u64_at(self.bytes, 0x30) } else { None }
    }

    /// Espacio en disco de un atributo no residente (válido si `lowest_vcn == 0`).
    /// En los comprimidos o dispersos se usa el total realmente asignado.
    pub fn on_disk(&self) -> Option<u64> {
        if !self.non_resident {
            return None;
        }
        let allocated = u64_at(self.bytes, 0x28)?;
        if self.flags & (ATTR_FLAG_COMPRESSED | ATTR_FLAG_SPARSE) != 0 && self.bytes.len() >= 0x48 {
            return u64_at(self.bytes, 0x40).or(Some(allocated));
        }
        Some(allocated)
    }

    /// Tramos de clústeres de un atributo no residente.
    pub fn runs(&self) -> Option<Vec<Run>> {
        let off = u16_at(self.bytes, 0x20)? as usize;
        decode_runs(self.bytes.get(off..)?, self.lowest_vcn()?)
    }
}

/// Recorre los atributos de un registro ya corregido con [`apply_fixup`].
pub(crate) fn attributes(rec: &[u8]) -> impl Iterator<Item = Attr<'_>> {
    let used = (u32_at(rec, 0x18).unwrap_or(0) as usize).min(rec.len());
    let mut off = u16_at(rec, 0x14).unwrap_or(u16::MAX) as usize;
    std::iter::from_fn(move || {
        if off + 16 > used {
            return None;
        }
        let ty = u32_at(rec, off)?;
        let len = u32_at(rec, off + 4)? as usize;
        if ty == ATTR_END || len < 16 || off + len > used {
            return None;
        }
        let bytes = &rec[off..off + len];
        off += len;
        Some(Attr { ty, non_resident: bytes[8] != 0, name_len: bytes[9], flags: u16_at(bytes, 0x0C)?, bytes })
    })
}

/// Tramo de clústeres contiguos: `len` clústeres desde `lcn` en disco que
/// corresponden a los VCN `vcn..vcn+len` del atributo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Run {
    pub vcn: u64,
    pub lcn: u64,
    pub len: u64,
}

/// Decodifica los "mapping pairs" de un atributo no residente. Los tramos
/// dispersos (sin clústeres en disco) se omiten.
pub(crate) fn decode_runs(data: &[u8], mut vcn: u64) -> Option<Vec<Run>> {
    let mut out = Vec::new();
    let mut pos = 0;
    let mut lcn: i64 = 0;
    loop {
        let header = *data.get(pos)?;
        pos += 1;
        if header == 0 {
            break;
        }
        let len_size = (header & 0x0F) as usize;
        let off_size = (header >> 4) as usize;
        if len_size == 0 || len_size > 8 || off_size > 8 {
            return None;
        }
        let len = le_unsigned(data.get(pos..pos + len_size)?);
        pos += len_size;
        if off_size > 0 {
            lcn = lcn.checked_add(le_signed(data.get(pos..pos + off_size)?))?;
            pos += off_size;
            if lcn < 0 {
                return None;
            }
            out.push(Run { vcn, lcn: lcn as u64, len });
        }
        vcn = vcn.checked_add(len)?;
    }
    Some(out)
}

fn le_unsigned(b: &[u8]) -> u64 {
    b.iter().rev().fold(0u64, |acc, &x| (acc << 8) | u64::from(x))
}

fn le_signed(b: &[u8]) -> i64 {
    let shift = 64 - b.len() * 8;
    if shift == 0 { le_unsigned(b) as i64 } else { ((le_unsigned(b) << shift) as i64) >> shift }
}

/// Analiza registros consecutivos (`data` empieza en el registro `first_frn`)
/// y rellena `recs`, que cubre ese mismo rango.
pub(crate) fn parse_records(
    data: &mut [u8],
    rec_size: usize,
    first_frn: u64,
    recs: &mut [RecInfo],
    out: &mut ParseOut,
) {
    for (i, (rec, slot)) in data.chunks_exact_mut(rec_size).zip(recs.iter_mut()).enumerate() {
        parse_record(rec, first_frn + i as u64, slot, out);
    }
}

/// Analiza un registro. Los registros base se escriben en `slot`; los de
/// extensión se guardan en `out.ext` para unirlos después a su base.
pub(crate) fn parse_record(rec: &mut [u8], frn: u64, slot: &mut RecInfo, out: &mut ParseOut) {
    if rec.get(0..4) != Some(b"FILE") || !apply_fixup(rec) {
        return;
    }
    let header_flags = u16_at(rec, 0x16).unwrap_or(0);
    if header_flags & RECORD_IN_USE == 0 {
        return;
    }
    let base = u64_at(rec, 0x20).unwrap_or(0) & FRN_MASK;
    let target = if base == 0 { frn } else { base };
    if target > u64::from(u32::MAX) {
        return;
    }

    let mut info = RecInfo::default();
    let mut size = None;
    let mut alloc = 0u64;
    let mut tag = 0u32;
    for a in attributes(rec) {
        if a.non_resident {
            if a.lowest_vcn() == Some(0) {
                alloc += a.on_disk().unwrap_or(0);
                if a.ty == ATTR_DATA && a.name_len == 0 {
                    size = a.data_size();
                }
            }
            continue;
        }
        let Some(value) = a.value() else { continue };
        match a.ty {
            ATTR_STANDARD_INFORMATION if base == 0 => {
                info.mtime = u64_at(value, 8).map_or(0, |t| filetime_to_unix(t as i64));
                info.attrs = u32_at(value, 32).unwrap_or(0);
            }
            ATTR_FILE_NAME => parse_file_name(value, target as u32, &mut tag, out),
            ATTR_DATA if a.name_len == 0 => size = Some(value.len() as u64),
            _ => {}
        }
    }

    if base == 0 {
        info.flags = IN_USE | if header_flags & RECORD_IS_DIRECTORY != 0 { IS_DIR } else { 0 };
        info.size = size.unwrap_or(0);
        info.alloc = alloc;
        info.tag = tag;
        if info.flags & IS_DIR != 0 {
            out.dirs += 1;
        } else {
            out.files += 1;
        }
        out.bytes += alloc;
        *slot = info;
    } else {
        out.ext.push(ExtPart { base: target as u32, size, alloc, tag });
    }
}

/// Lee un atributo $FILE_NAME y añade el nombre como enlace.
fn parse_file_name(value: &[u8], target: u32, tag: &mut u32, out: &mut ParseOut) {
    let (Some(parent), Some(&chars), Some(&namespace)) = (u64_at(value, 0), value.get(64), value.get(65)) else {
        return;
    };
    if namespace == NAMESPACE_DOS {
        return;
    }
    let attrs = u32_at(value, 56).unwrap_or(0);
    if attrs & 0x400 != 0 {
        *tag = u32_at(value, 60).unwrap_or(0);
    }
    let Some(raw) = value.get(66..66 + chars as usize * 2) else { return };
    let parent = parent & FRN_MASK;
    if parent > u64::from(u32::MAX) {
        return;
    }
    let mut wide = [0u16; 255];
    for (w, b) in wide.iter_mut().zip(raw.chunks_exact(2)) {
        *w = u16::from_le_bytes([b[0], b[1]]);
    }
    let start = out.names.len();
    push_name(&mut out.names, &wide[..chars as usize]);
    out.links.push(Link {
        frn: target,
        parent: parent as u32,
        name_off: start as u32,
        name_len: (out.names.len() - start).min(u16::MAX as usize) as u16,
    });
}

/// Segmentos (registros) que contienen tramos del flujo $DATA sin nombre,
/// según un $ATTRIBUTE_LIST. Se usa para reconstruir una MFT fragmentada.
pub(crate) fn data_segments_in_list(list: &[u8]) -> Vec<u64> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos + 26 <= list.len() {
        let (Some(ty), Some(len)) = (u32_at(list, pos), u16_at(list, pos + 4)) else { break };
        if len == 0 {
            break;
        }
        let name_len = list[pos + 6];
        let segment = u64_at(list, pos + 16).unwrap_or(0) & FRN_MASK;
        if ty == ATTR_DATA && name_len == 0 && segment != 0 {
            out.push(segment);
        }
        pos += len as usize;
    }
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Construye registros FILE sintéticos como los escribe NTFS.
    pub(crate) struct RecordBuilder {
        buf: Vec<u8>,
        off: usize,
    }

    impl RecordBuilder {
        pub fn new(size: usize, flags: u16, base: u64) -> RecordBuilder {
            let mut buf = vec![0u8; size];
            buf[0..4].copy_from_slice(b"FILE");
            let usa_count = (size / USA_STRIDE + 1) as u16;
            buf[4..6].copy_from_slice(&0x30u16.to_le_bytes());
            buf[6..8].copy_from_slice(&usa_count.to_le_bytes());
            buf[0x14..0x16].copy_from_slice(&0x38u16.to_le_bytes());
            buf[0x16..0x18].copy_from_slice(&flags.to_le_bytes());
            buf[0x1C..0x20].copy_from_slice(&(size as u32).to_le_bytes());
            buf[0x20..0x28].copy_from_slice(&base.to_le_bytes());
            RecordBuilder { buf, off: 0x38 }
        }

        fn header(&mut self, ty: u32, len: usize, non_resident: bool, name: &str, flags: u16) -> usize {
            let len = len.next_multiple_of(8);
            let o = self.off;
            self.buf[o..o + 4].copy_from_slice(&ty.to_le_bytes());
            self.buf[o + 4..o + 8].copy_from_slice(&(len as u32).to_le_bytes());
            self.buf[o + 8] = u8::from(non_resident);
            self.buf[o + 9] = name.encode_utf16().count() as u8;
            self.buf[o + 0x0C..o + 0x0E].copy_from_slice(&flags.to_le_bytes());
            self.off += len;
            o
        }

        pub fn resident(mut self, ty: u32, name: &str, value: &[u8]) -> Self {
            let name_bytes: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
            let value_off = (0x18 + name_bytes.len()).next_multiple_of(8);
            let o = self.header(ty, value_off + value.len(), false, name, 0);
            self.buf[o + 0x0A..o + 0x0C].copy_from_slice(&0x18u16.to_le_bytes());
            self.buf[o + 0x18..o + 0x18 + name_bytes.len()].copy_from_slice(&name_bytes);
            self.buf[o + 0x10..o + 0x14].copy_from_slice(&(value.len() as u32).to_le_bytes());
            self.buf[o + 0x14..o + 0x16].copy_from_slice(&(value_off as u16).to_le_bytes());
            self.buf[o + value_off..o + value_off + value.len()].copy_from_slice(value);
            self
        }

        #[allow(clippy::too_many_arguments)]
        pub fn non_resident(
            mut self,
            ty: u32,
            name: &str,
            flags: u16,
            lowest_vcn: u64,
            allocated: u64,
            size: u64,
            total_allocated: u64,
            runs: &[u8],
        ) -> Self {
            let name_bytes: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
            let name_off = 0x48;
            let runs_off = (name_off + name_bytes.len()).next_multiple_of(8);
            let o = self.header(ty, runs_off + runs.len() + 1, true, name, flags);
            self.buf[o + 0x0A..o + 0x0C].copy_from_slice(&(name_off as u16).to_le_bytes());
            self.buf[o + 0x10..o + 0x18].copy_from_slice(&lowest_vcn.to_le_bytes());
            self.buf[o + 0x20..o + 0x22].copy_from_slice(&(runs_off as u16).to_le_bytes());
            self.buf[o + 0x28..o + 0x30].copy_from_slice(&allocated.to_le_bytes());
            self.buf[o + 0x30..o + 0x38].copy_from_slice(&size.to_le_bytes());
            self.buf[o + 0x38..o + 0x40].copy_from_slice(&size.to_le_bytes());
            self.buf[o + 0x40..o + 0x48].copy_from_slice(&total_allocated.to_le_bytes());
            self.buf[o + name_off..o + name_off + name_bytes.len()].copy_from_slice(&name_bytes);
            self.buf[o + runs_off..o + runs_off + runs.len()].copy_from_slice(runs);
            self
        }

        pub fn std_info(self, mtime_filetime: u64, attrs: u32) -> Self {
            let mut v = vec![0u8; 72];
            v[8..16].copy_from_slice(&mtime_filetime.to_le_bytes());
            v[32..36].copy_from_slice(&attrs.to_le_bytes());
            self.resident(ATTR_STANDARD_INFORMATION, "", &v)
        }

        pub fn file_name(self, parent: u64, name: &str, namespace: u8) -> Self {
            self.file_name_tagged(parent, name, namespace, 0, 0)
        }

        pub fn file_name_tagged(self, parent: u64, name: &str, namespace: u8, attrs: u32, tag: u32) -> Self {
            let units: Vec<u16> = name.encode_utf16().collect();
            let mut v = vec![0u8; 66 + units.len() * 2];
            v[0..8].copy_from_slice(&(parent | (1 << 48)).to_le_bytes());
            v[56..60].copy_from_slice(&attrs.to_le_bytes());
            v[60..64].copy_from_slice(&tag.to_le_bytes());
            v[64] = units.len() as u8;
            v[65] = namespace;
            for (i, u) in units.iter().enumerate() {
                v[66 + i * 2..68 + i * 2].copy_from_slice(&u.to_le_bytes());
            }
            self.resident(ATTR_FILE_NAME, "", &v)
        }

        /// Cierra el registro y aplica la protección de la update sequence.
        pub fn build(mut self) -> Vec<u8> {
            let o = self.off;
            self.buf[o..o + 4].copy_from_slice(&ATTR_END.to_le_bytes());
            self.buf[0x18..0x1C].copy_from_slice(&((o + 8) as u32).to_le_bytes());
            let count = self.buf.len() / USA_STRIDE + 1;
            let check = [0x5A, 0xA5];
            self.buf[0x30..0x32].copy_from_slice(&check);
            for i in 1..count {
                let p = i * USA_STRIDE - 2;
                let saved = [self.buf[p], self.buf[p + 1]];
                self.buf[0x30 + i * 2..0x32 + i * 2].copy_from_slice(&saved);
                self.buf[p..p + 2].copy_from_slice(&check);
            }
            self.buf
        }
    }

    pub(crate) const FT_2020: u64 = 132_223_104_000_000_000; // 2020-01-01

    fn parse(rec: &mut [u8], frn: u64) -> (RecInfo, ParseOut) {
        let mut slot = RecInfo::default();
        let mut out = ParseOut::default();
        parse_record(rec, frn, &mut slot, &mut out);
        (slot, out)
    }

    #[test]
    fn fixup_restaura_y_detecta_danos() {
        let mut rec = RecordBuilder::new(1024, RECORD_IN_USE, 0).std_info(FT_2020, 0).build();
        let original_tail = [rec[510], rec[511]];
        assert_eq!(original_tail, [0x5A, 0xA5], "protegido");
        let mut ok = rec.clone();
        assert!(apply_fixup(&mut ok));
        assert_ne!([ok[510], ok[511]], [0x5A, 0xA5]);
        rec[1022] = 0; // escritura a medias
        assert!(!apply_fixup(&mut rec));
    }

    #[test]
    fn tramos_de_clusteres() {
        // 0x21: longitud 1 byte, desplazamiento 2 bytes. Luego un tramo hacia atrás y uno disperso.
        let runs = [0x21, 0x10, 0x00, 0x01, 0x11, 0x08, 0xF0, 0x01, 0x05, 0x00];
        let r = decode_runs(&runs, 0).unwrap();
        assert_eq!(
            r,
            vec![Run { vcn: 0, lcn: 0x100, len: 0x10 }, Run { vcn: 0x10, lcn: 0x100 - 0x10, len: 8 }],
            "el tramo disperso de 5 clústeres no aparece"
        );
        assert!(decode_runs(&[0x21, 0x10], 0).is_none(), "datos truncados");
    }

    #[test]
    fn registro_de_archivo() {
        let mut rec = RecordBuilder::new(1024, RECORD_IN_USE, 0)
            .std_info(FT_2020, 0x2 | 0x20)
            .file_name(40, "LEEME~1.TXT", NAMESPACE_DOS)
            .file_name(40, "Léeme ahora.txt", 1)
            .non_resident(ATTR_DATA, "", 0, 0, 8192, 5000, 0, &[0x11, 0x02, 0x40, 0x00])
            .resident(ATTR_DATA, "Zone.Identifier", b"[ZoneTransfer]")
            .build();
        let (info, out) = parse(&mut rec, 77);
        assert_eq!(info.flags, IN_USE);
        assert_eq!(info.size, 5000);
        assert_eq!(info.alloc, 8192);
        assert_eq!(info.attrs, 0x22);
        assert_eq!(info.mtime, 1_577_836_800);
        assert_eq!(out.links.len(), 1, "el nombre DOS se ignora");
        let l = out.links[0];
        assert_eq!((l.frn, l.parent), (77, 40));
        assert_eq!(&out.names[l.name_off as usize..][..l.name_len as usize], "Léeme ahora.txt".as_bytes());
        assert_eq!((out.files, out.dirs), (1, 0));
    }

    #[test]
    fn archivo_residente_disperso_y_flujos() {
        let mut small = RecordBuilder::new(1024, RECORD_IN_USE, 0)
            .std_info(FT_2020, 0)
            .file_name(5, "nota.txt", 3)
            .resident(ATTR_DATA, "", b"hola mundo")
            .build();
        let (info, _) = parse(&mut small, 30);
        assert_eq!((info.size, info.alloc), (10, 0));

        // Disperso: reservado 1 GB pero solo 64 KB asignados de verdad.
        let mut sparse = RecordBuilder::new(1024, RECORD_IN_USE, 0)
            .std_info(FT_2020, 0x200)
            .file_name(5, "disperso.bin", 1)
            .non_resident(ATTR_DATA, "", ATTR_FLAG_SPARSE, 0, 1 << 30, 1 << 30, 65536, &[0x01, 0x10])
            .non_resident(ATTR_DATA, "WofCompressedData", 0, 0, 4096, 3000, 0, &[0x11, 0x01, 0x20])
            .build();
        let (info, _) = parse(&mut sparse, 31);
        assert_eq!(info.size, 1 << 30);
        assert_eq!(info.alloc, 65536 + 4096, "suma el flujo alternativo en disco");
    }

    #[test]
    fn carpeta_y_registro_de_extension() {
        let mut dir = RecordBuilder::new(1024, RECORD_IN_USE | RECORD_IS_DIRECTORY, 0)
            .std_info(FT_2020, 0x10)
            .file_name(5, "Juegos", 1)
            .non_resident(0xA0, "$I30", 0, 0, 4096, 4096, 0, &[0x11, 0x01, 0x30])
            .build();
        let (info, out) = parse(&mut dir, 50);
        assert_eq!(info.flags, IN_USE | IS_DIR);
        assert_eq!(info.alloc, 4096, "el índice de la carpeta ocupa disco");
        assert_eq!((out.files, out.dirs), (0, 1));

        // Registro de extensión con el primer tramo del $DATA y otro nombre.
        let mut ext = RecordBuilder::new(1024, RECORD_IN_USE, 90)
            .file_name(50, "otro nombre.dat", 1)
            .non_resident(ATTR_DATA, "", 0, 0, 1 << 20, 1_000_000, 0, &[0x11, 0x01, 0x30])
            .build();
        let mut slot = RecInfo::default();
        let mut out = ParseOut::default();
        parse_record(&mut ext, 1234, &mut slot, &mut out);
        assert_eq!(slot, RecInfo::default(), "la extensión no escribe en su propio hueco");
        assert_eq!(out.ext, vec![ExtPart { base: 90, size: Some(1_000_000), alloc: 1 << 20, tag: 0 }]);
        assert_eq!(out.links[0].frn, 90, "el nombre pertenece al registro base");
    }

    #[test]
    fn union_con_etiqueta() {
        let mut rec = RecordBuilder::new(1024, RECORD_IN_USE | RECORD_IS_DIRECTORY, 0)
            .std_info(FT_2020, 0x400 | 0x10)
            .file_name_tagged(5, "Documents and Settings", 1, 0x400, 0xA000_0003)
            .build();
        let (info, _) = parse(&mut rec, 60);
        assert_eq!(info.tag, 0xA000_0003);
    }

    #[test]
    fn registros_libres_o_danados_se_ignoran() {
        let mut free = RecordBuilder::new(1024, 0, 0).std_info(FT_2020, 0).file_name(5, "borrado", 1).build();
        let (info, out) = parse(&mut free, 70);
        assert_eq!(info, RecInfo::default());
        assert!(out.links.is_empty());

        let mut basura = vec![0xFFu8; 1024];
        let (info, out) = parse(&mut basura, 71);
        assert_eq!(info, RecInfo::default());
        assert!(out.links.is_empty());
    }

    #[test]
    fn lista_de_atributos() {
        let mut list = Vec::new();
        for (ty, segment, name_len) in
            [(0x10u32, 0u64, 0u8), (0x80, 0, 0), (0x80, 900, 0), (0x80, 901, 4), (0x80, 900, 0)]
        {
            let mut e = vec![0u8; 32];
            e[0..4].copy_from_slice(&ty.to_le_bytes());
            e[4..6].copy_from_slice(&32u16.to_le_bytes());
            e[6] = name_len;
            e[16..24].copy_from_slice(&(segment | (3 << 48)).to_le_bytes());
            list.extend(e);
        }
        assert_eq!(data_segments_in_list(&list), vec![900]);
    }

    #[test]
    fn varios_bloques_se_unen() {
        let mut a = ParseOut::default();
        let mut b = ParseOut::default();
        let mut slot = RecInfo::default();
        parse_record(&mut RecordBuilder::new(1024, 1, 0).file_name(5, "a", 1).build(), 20, &mut slot, &mut a);
        parse_record(&mut RecordBuilder::new(1024, 1, 0).file_name(5, "bb", 1).build(), 21, &mut slot, &mut b);
        a.append(b);
        assert_eq!(a.links.len(), 2);
        let l = a.links[1];
        assert_eq!(&a.names[l.name_off as usize..][..l.name_len as usize], b"bb");
        assert_eq!(a.files, 2);
    }
}
