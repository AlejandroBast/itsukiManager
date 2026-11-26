# itsukiManager

Gestor de almacenamiento para Windows: escanea discos completos en segundos y
muestra qué ocupa espacio (carpetas de más de 1 GB, archivos grandes, tipos de
archivo) para poder liberar espacio con seguridad.

> En desarrollo. Escrito en Rust.

## Estructura

| Carpeta | Contenido |
|---|---|
| `crates/itsuki-core` | Núcleo: motor de escaneo, árbol compacto, consultas y acciones seguras |
| `crates/itsuki-cli` | Programa de línea de comandos `itsuki` |

## Compilar

Requisitos: [Rust](https://rustup.rs) estable y Visual Studio Build Tools (C++).

```powershell
cargo build --release
```
