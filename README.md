# itsukiManager

Gestor de almacenamiento para Windows, escrito en Rust. Escanea discos completos
en segundos y muestra qué ocupa espacio (carpetas de más de 1 GB, archivos
grandes, tipos de archivo) para liberar espacio con seguridad.

> **Estado:** fases 1 a 4 listas: motores de escaneo nativo y MFT, limpieza
> inteligente, duplicados, historial con avisos y línea de comandos `itsuki`. En curso:
> la interfaz de escritorio (inicio, análisis, archivos grandes, duplicados y limpieza listos).

## Qué hace

- **Carpetas grandes**: todas las que superan un tamaño (1 GB por defecto) o solo
  las "hojas", es decir, las que contienen de verdad el espacio.
- **Archivos más grandes**, con filtro por antigüedad ("sin tocar en un año").
- **Espacio por tipo**: vídeo, juegos, discos virtuales, caché, instaladores…
- **Árbol** de carpetas con tamaños y porcentajes.
- **Informe** de una sola pasada con lo más importante.
- **Limpieza inteligente**: encuentra temporales, cachés de navegadores, aplicaciones y
  GPU, descargas de Windows Update, volcados de errores, `node_modules`, compilaciones,
  cachés de npm/pip/cargo, cachés de Steam y Epic, la Papelera e instaladores viejos,
  cada uno con su nivel de seguridad, y los limpia sin riesgo.
- **Duplicados**: compara el contenido real de los archivos (no solo el nombre).
- **Historial y crecimiento**: guarda fotos de cada escaneo y muestra qué carpetas
  crecieron o se redujeron desde la anterior.
- **Avisos**: unidades casi llenas y a qué ritmo se llenan (`se llena en ~20 días`).
- **Papelera segura**: `papelera` nunca borra de forma permanente y rechaza rutas
  críticas (raíz de la unidad, Windows, Archivos de programa, carpetas personales completas…).
- **Exportación** a CSV (separado por `;`, se abre directo en Excel en español) y JSON.

## Limpieza inteligente

`itsuki limpieza` analiza la unidad y agrupa lo que encuentra por categoría y nivel:

| Nivel | Qué significa | Ejemplos |
|---|---|---|
| **Seguro** | Se regenera solo, sin coste | Temporales (de más de 24 h), cachés de navegadores y apps, cachés de sombreadores de la GPU, volcados de errores, descargas de Windows Update |
| **Regenerable** | Se vuelve a crear o descargar, pero cuesta tiempo | `node_modules`, `target`, `bin/obj`, `__pycache__`, cachés de npm, pip, cargo, Gradle |
| **Revisar** | Puede contener cosas tuyas | Papelera, descargas incompletas de Steam, instaladores de hace más de 30 días en Descargas |
| **Consejo** | No se borra desde aquí: se explica cómo hacerlo | Disco virtual de Docker o WSL, WinSxS, `hiberfil.sys`, `Windows.old`, puntos de restauración |

Medido en el PC de desarrollo (C:, sin administrador): **14,5 GB seguros**, 2,3 GB
regenerables y 1,6 GB para revisar, más 43,7 GB en sugerencias (Docker, WinSxS,
hibernación). La primera limpieza real de lo seguro (con Brave, Discord y otras apps
abiertas, que se saltaron) liberó **10,1 GB en 15 768 archivos en 13 s**.

Reglas de seguridad al borrar:

- Nunca sigue enlaces simbólicos ni uniones: borra el enlace, no lo que hay detrás.
- No toca archivos de OneDrive que solo están en la nube ni otros puntos de reparse.
- Los patrones de proyectos (`node_modules`, `target`…) no se aplican dentro de
  programas instalados (AppData, Archivos de programa, extensiones de VS Code…).
- Se saltan las cachés de las aplicaciones abiertas (navegador, Discord, Steam…): ciérralas
  y repite, o usa `--incluir-abiertas`.
- Lo que esté en uso o sin permiso se salta y se informa.
- Pide confirmación y avisa de que el borrado es permanente; con `--papelera` se
  envía a la Papelera.

`itsuki duplicados` agrupa por tamaño, compara después un hash del principio y del
final de cada archivo y solo lee entero lo que sigue empatado (xxHash3 de 128 bits).
Sin ruta revisa tus carpetas personales; se salta los enlaces duros, los archivos en
la nube, los programas y las bibliotecas de juegos, donde los repetidos son normales.

## Historial y avisos

`itsuki crecimiento` escanea, guarda una foto y la compara con la anterior de la
misma ruta (o con la que indiques: `--desde 12`, `--desde 7d`, `--desde 2026-11-20`).
Muestra solo las carpetas que explican el cambio: si `Docker\wsl\disk` creció 3 GB, no
repite también `AppData`, `Local` y `Docker` (`--todas` las incluye).

- Las fotos se guardan en SQLite en `%LOCALAPPDATA%\itsukiManager\historial.db`
  (o en la carpeta de la variable `ITSUKI_DATOS`).
- Cada foto guarda los totales, el espacio libre de la unidad y las carpetas de
  100 MB o más (unos 80 KB por foto de todo C:). Se conservan las 100 más recientes
  de cada ruta.

`itsuki avisos` revisa todas las unidades:

| Estado | Cuándo |
|---|---|
| **Crítico** | Queda menos del mayor entre 5 GB y el 5 % del disco |
| **Aviso** | Queda menos del mayor entre 20 GB y el 10 % (ajustable con `--min-libre 15%` o `--min-libre 50GB`), o al ritmo actual se llena en menos de 30 días |

El ritmo de llenado se calcula con las fotos del historial de los últimos 30 días
(cualquier escaneo de esa unidad sirve). Si hay algún aviso termina con código de
salida 1, para usarlo en scripts. Las notificaciones automáticas llegarán con la
interfaz gráfica (icono en la bandeja del sistema).

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
itsuki limpieza                          # basura conocida por categoría y nivel
itsuki limpieza --limpiar seguro         # limpia todo lo seguro (pide confirmación)
itsuki limpieza --limpiar temporales,papelera --papelera
itsuki duplicados                        # duplicados en tus carpetas personales
itsuki duplicados D:\ --min 10MB --csv duplicados.csv
itsuki crecimiento                       # guarda una foto y muestra qué creció
itsuki crecimiento --desde 7d           # comparado con hace una semana
itsuki historial                         # fotos guardadas (historial borrar 3)
itsuki avisos                            # unidades casi llenas y tendencia
```

Para el motor MFT, abre la terminal como administrador: `itsuki` lo usa solo al
escanear una unidad completa (o fuérzalo con `--motor mft`).

Opciones comunes: `--metrica disco|datos`, `--hilos N`, `--motor auto|mft|nativo|portable`,
`--contar-enlaces-duros`, `-q` (sin progreso). Ayuda completa: `itsuki --help`.

## Interfaz de escritorio

La app está en `app/`: React + TypeScript con Vite para la interfaz y Tauri 2
(`app/src-tauri`) como capa fina que llama al núcleo. El diseño de referencia está en
[`docs/ui/mockup.png`](docs/ui/mockup.png).

```powershell
cd app
npm install
npm run tauri dev      # abre la app con recarga en caliente
npm run tauri build    # genera el instalador (NSIS)
```

- La ventana tiene un tamaño fijo de 1672 × 941, el del diseño (1 rem = 16 px): no se
  redimensiona ni se maximiza. Si la pantalla es más pequeña (por ejemplo, un portátil con
  escalado alto), se reduce sin perder la proporción.
- La pantalla de inicio muestra datos reales: la unidad del sistema, el reparto del
  espacio, las carpetas más grandes y lo que se puede liberar.
- Cada sección funciona con botones, sin terminal. El escaneo de la unidad se hace una vez y
  lo reutilizan todas las secciones; los trabajos largos muestran el progreso y se pueden cancelar.
  - **Analizar disco**: árbol de las carpetas de 500 MB, 1, 5 o 10 GB o más de cada unidad.
  - **Archivos grandes**: los 50, 100 o 200 archivos que más ocupan.
  - **Duplicados**: compara el contenido en las carpetas personales o en una unidad y deja una
    sola copia (la más antigua).
  - **Limpieza**: temporales y cachés por categoría y nivel de seguridad; lo seguro viene marcado.
    Las cachés de apps abiertas y lo que necesita administrador se saltan.
- Todo lo que borra pide confirmación. Desde las listas se envía a la Papelera, y las rutas
  protegidas (Windows, programas, carpetas base del usuario) se rechazan. La limpieza borra de
  forma permanente salvo que se marque «Enviar a la Papelera».
- Las imágenes de la mascota están inspiradas en Itsuki Nakano, personaje de terceros:
  el paquete de assets no concede licencia sobre el personaje, así que conviene revisar
  los derechos antes de distribuir la app públicamente.

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
| `crates/itsuki-core` | Núcleo reutilizable: escaneo (`scan`, con los motores `mft`, `native` y `portable`), árbol (`tree`), consultas (`query`), tipos (`types`), limpieza (`clean`), duplicados (`dupes`), historial (`history`), avisos (`alerts`), acciones seguras (`actions`), sistema (`system`) |
| `crates/itsuki-cli` | Programa de línea de comandos `itsuki` |
| `app` | Interfaz de escritorio (React + TypeScript + Vite) y `app/src-tauri` (Tauri 2) |
| `docs/ui` | Mockup de referencia de la interfaz |

## Hoja de ruta

1. ~~Motor de escaneo paralelo y CLI~~
2. ~~Motor MFT para NTFS~~
3. ~~Limpieza inteligente y duplicados~~
4. ~~Historial de escaneos, crecimiento por carpeta y avisos de disco lleno~~
5. Interfaz gráfica (Tauri + React) con treemap interactivo
