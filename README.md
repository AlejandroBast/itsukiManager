# itsukiManager

Gestor de almacenamiento para Windows, escrito en Rust. Escanea discos completos
en segundos y muestra qué ocupa espacio (carpetas de más de 1 GB, archivos
grandes, tipos de archivo) para liberar espacio con seguridad.

> **Estado:** fase 1 lista (motor de escaneo + línea de comandos `itsuki`).
> La interfaz gráfica llegará en una fase posterior.

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

## Por qué es rápido y preciso

| Técnica | Efecto |
|---|---|
| API nativa: `NtCreateFile` relativo al padre + `GetFileInformationByHandleEx` en bloques de 64 KiB | Una llamada devuelve cientos de entradas con tamaño, atributos e ID; sin límite de 260 caracteres |
| Paralelismo con robo de trabajo (rayon) | Todas las ramas del disco se reparten entre todos los núcleos |
| Árbol compacto: 48 bytes por entrada y nombres en un solo búfer | Escanear todo C: (600 000 archivos) usa unos 85 MB de RAM en total |
| Tamaño "en disco" además del tamaño de datos | Los archivos de OneDrive solo en la nube no cuentan como espacio usado |
| Enlaces duros y vistas del mismo archivo contados una sola vez | `C:\Windows\WinSxS` aparece con su tamaño real (13 GB en vez de 22 GB) |
| No sigue uniones ni enlaces simbólicos | Nada se cuenta dos veces |
| Como administrador activa `SeBackupPrivilege` | Lee también carpetas protegidas del sistema |

### Rendimiento medido

Disco C: NVMe de 2 TB con 619 GB usados, 598 565 archivos y 111 776 carpetas
(Ryzen 5 4600G, 12 hilos):

| Herramienta | Tiempo |
|---|---|
| Script PowerShell + .NET (prototipo) | 17,6 s |
| `itsuki` (primer escaneo) | 9,2 s |
| `itsuki` (con la caché del sistema caliente) | ~5 s |

Sin permisos de administrador el límite es el propio Windows: el 95 % del tiempo
de CPU se va en el kernel (el filtro del antivirus revisa cada carpeta que se abre). La
siguiente fase añade un motor que lee directamente la tabla maestra de archivos
(MFT) de NTFS, que evita ese coste.

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

Opciones comunes: `--metrica disco|datos`, `--hilos N`, `--motor auto|nativo|portable`,
`--contar-enlaces-duros`, `-q` (sin progreso). Ayuda completa: `itsuki --help`.

## Compilar y probar

Requisitos: [Rust](https://rustup.rs) estable y Visual Studio Build Tools (C++).

```powershell
cargo build --release        # genera target\release\itsuki.exe (~1,2 MB)
cargo test                   # pruebas unitarias y de integración
cargo clippy --all-targets
```

## Estructura

| Carpeta | Contenido |
|---|---|
| `crates/itsuki-core` | Núcleo reutilizable: escaneo (`scan`), árbol (`tree`), consultas (`query`), tipos (`types`), acciones seguras (`actions`), sistema (`system`) |
| `crates/itsuki-cli` | Programa de línea de comandos `itsuki` |

## Hoja de ruta

1. ~~Motor de escaneo paralelo y CLI~~
2. Motor MFT para NTFS (escaneo completo en 1–3 s como administrador)
3. Limpieza inteligente: basura conocida (Temp, cachés, `node_modules`, Windows Update…) y duplicados
4. Historial de escaneos, crecimiento por carpeta y avisos de disco lleno
5. Interfaz gráfica (Tauri + React) con treemap interactivo
