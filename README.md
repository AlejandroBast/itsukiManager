# itsukiManager

Gestor de almacenamiento para Windows, escrito en Rust. Escanea discos completos
en segundos y muestra qué ocupa espacio (carpetas de más de 1 GB, archivos
grandes, tipos de archivo) para liberar espacio con seguridad.

> **Estado:** fases 1 y 2 listas: motor de escaneo nativo, motor MFT de NTFS y
> línea de comandos `itsuki`. La interfaz gráfica llegará en una fase posterior.

## Qué hace

- **Carpetas grandes**: todas las que superan un tamaño (1 GB por defecto) o solo
  las "hojas", es decir, las que contienen de verdad el espacio.
- **Archivos más grandes**, con filtro por antigüedad ("sin tocar en un año").
- **Espacio por tipo**: vídeo, juegos, discos virtuales, caché, instaladores…
- **Árbol** de carpetas con tamaños y porcentajes.
- **Informe** de una sola pasada con lo más importante.
- **Papelera segura**: nunca borra de forma permanente y rechaza rutas críticas
  (raíz de la unidad, Windows, Archivos de programa, carpetas personales completas…).
- **Exportación** a CSV (separado por `;`, se abre directo en Excel en español) y JSON.

## Motores de escaneo

| Motor | Cuándo se usa | Cómo funciona |
|---|---|---|
| `mft` | Automático como administrador al escanear una unidad NTFS completa | Abre el volumen una sola vez y lee la tabla maestra de archivos (MFT) en bloques de 4 MiB con varios lectores en paralelo, mientras 12 hilos analizan los registros |
| `nativo` | Automático en el resto de casos | `NtCreateFile` relativo al padre + `GetFileInformationByHandleEx` en bloques de 64 KiB, con robo de trabajo entre hilos |
| `portable` | Solo a mano (`--motor portable`) | Biblioteca estándar de Rust; sirve de referencia |

El motor MFT además:

- ve **todas** las carpetas, también las protegidas del sistema y de otros usuarios;
- saca los tamaños del atributo de datos (siempre exactos) y el espacio real de
  cada archivo a partir de sus tramos de clústeres, incluidos los comprimidos,
  los dispersos, los flujos alternativos y los índices de carpeta;
- muestra los archivos internos de NTFS (`$MFT`, `$LogFile`, `$Extend`…);
- si algo falla (volumen no NTFS, de red, o protegido) vuelve solo al motor nativo.

Las bibliotecas de juegos de Xbox (unidades con `XboxGames` y `.GamingRoot`) tienen
un filtro de Gaming Services que bloquea la lectura directa: ahí se usa el motor nativo.

## Por qué es preciso

| Técnica | Efecto |
|---|---|
| Tamaño "en disco" además del tamaño de datos | Los archivos de OneDrive solo en la nube no cuentan como espacio usado |
| Enlaces duros y vistas del mismo archivo contados una sola vez | `C:\Windows\WinSxS` aparece con su tamaño real (13 GB en vez de 22 GB) |
| No sigue uniones ni enlaces simbólicos | Nada se cuenta dos veces |
| Árbol compacto: 48 bytes por entrada y nombres en un solo búfer | Todo C: (600 000 archivos) cabe en unos 85 MB de RAM |
| Como administrador activa `SeBackupPrivilege` | El motor nativo también lee las carpetas protegidas |

### Rendimiento medido

Disco C: NVMe de 2 TB con 670 GB usados, unos 658 000 archivos y 123 000 carpetas
(Ryzen 5 4600G, 12 hilos):

| Herramienta | Tiempo | Espacio sin contabilizar |
|---|---|---|
| Script PowerShell + .NET (prototipo) | 17,6 s | — |
| `itsuki`, motor nativo (usuario normal, primer escaneo) | 9,2 s | ~50 GB |
| `itsuki`, motor nativo (caché del sistema caliente) | ~5 s | ~50 GB |
| `itsuki`, **motor MFT** (administrador) | **1,5 s** | **20 MB** |

Con el motor MFT el 90 % del tiempo es leer los 713 MB de la MFT del disco;
analizar los 730 000 registros tarda 0,1 s. Sin administrador el límite es
Windows: el antivirus revisa cada carpeta que se abre y se lleva el 95 % del
tiempo de CPU.

## Uso

```powershell
itsuki carpetas                          # carpetas de 1 GB o más en C:\ (hasta 4 niveles)
itsuki carpetas D:\ --modo hojas --min 5GB
itsuki carpetas --csv informe.csv        # exportar para Excel
itsuki archivos --top 50                 # los 50 archivos más grandes
itsuki archivos --antiguos 365 --min 100MB
itsuki tipos C:\Users                    # espacio por categoría y extensión
itsuki arbol C:\Users -p 2               # árbol de 2 niveles
itsuki informe                           # resumen completo
itsuki papelera "C:\Users\yo\Downloads\viejo.iso"
itsuki abrir "C:\Program Files (x86)\Steam"
itsuki info                              # unidades (SSD/HDD), espacio y permisos
```

Para el motor MFT, abre la terminal como administrador: `itsuki` lo usa solo al
escanear una unidad completa (o fuérzalo con `--motor mft`).

Opciones comunes: `--metrica disco|datos`, `--hilos N`, `--motor auto|mft|nativo|portable`,
`--contar-enlaces-duros`, `-q` (sin progreso). Ayuda completa: `itsuki --help`.

## Compilar y probar

Requisitos: [Rust](https://rustup.rs) estable y Visual Studio Build Tools (C++).

```powershell
cargo build --release        # genera target\release\itsuki.exe (~1,2 MB)
cargo test                   # pruebas unitarias y de integración
cargo clippy --all-targets
```

La prueba `mft_coincide_con_nativo` solo se ejecuta como administrador; sin
permisos se omite.

### Herramientas de desarrollo

- `itsuki comparar [ruta]` (comando oculto): escanea con todos los motores
  disponibles y muestra las diferencias por carpeta.
- Variable `ITSUKI_PERFIL=1`: muestra en la salida de errores el tiempo de cada fase.

## Estructura

| Carpeta | Contenido |
|---|---|
| `crates/itsuki-core` | Núcleo reutilizable: escaneo (`scan`, con los motores `mft`, `native` y `portable`), árbol (`tree`), consultas (`query`), tipos (`types`), acciones seguras (`actions`), sistema (`system`) |
| `crates/itsuki-cli` | Programa de línea de comandos `itsuki` |

## Hoja de ruta

1. ~~Motor de escaneo paralelo y CLI~~
2. ~~Motor MFT para NTFS~~
3. Limpieza inteligente: basura conocida (Temp, cachés, `node_modules`, Windows Update…) y duplicados
4. Historial de escaneos, crecimiento por carpeta y avisos de disco lleno
5. Interfaz gráfica (Tauri + React) con treemap interactivo
