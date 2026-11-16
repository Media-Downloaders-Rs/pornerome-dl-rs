# Pornerome DL en Rust

Abre `abrir.cmd` o `pornerome-dl.exe`. Acepta varios enlaces de perfiles o posts. Los botones **Listar**, **Descargar**, **Actualizar** y **Cancelar** son independientes. Funciona sin Python ni navegador.

Guarda los medios directamente en `downloads/nombre-del-perfil/`, conservando los identificadores anteriores basados en la ruta del archivo. El seguimiento, los parciales y los respaldos de organización quedan dentro de `downloads/.pornerome-dl/`. Los cambios de servidor CDN no provocan descargas duplicadas; una reanudación entre servidores distintos reinicia de forma segura.

Se reutilizan los listados y la caché de posts de 24 horas. Las páginas nuevas se leen con hasta cuatro consultas concurrentes y separación entre peticiones. Los errores temporales tienen pausas compartidas y respetan Retry-After; los posts fallidos quedan pendientes sin descartar los correctos.

`organizar.cmd` y el botón **Organizar descargas anteriores** convierten las dos estructuras viejas. Exigen cerrar las demás instancias y conservan los conflictos y constancias archivadas. Un archivo incompleto no se marca como completo. La organización es explícita, no se inicia por descargar.

El repositorio anterior `../pornerome-dl` se conserva como respaldo. Se trasladaron **14.288 archivos (23,2 GB)** y toda la base a `downloads/`, comparando SHA-256 antes y después y verificando que Rust reconozca cada archivo como completo. Se recuperaron 25 listados y 7.942 posts; todas las filas de las tablas anteriores coinciden con la copia consistente de SQLite en `../pornerome-dl/.migration-backup/20261108-043707/`. Los lanzadores anteriores también abren esta versión Rust.

## Compilar

Este repositorio es independiente: incluye el motor, los módulos de los sitios y las pruebas. No requiere clonar otros repositorios. Cada repositorio mantiene su propia copia del motor; las correcciones comunes deben aplicarse a cada copia.

```powershell
cargo build --release --locked
# Ejecutar la aplicación recién compilada:
.\target\release\pornerome-dl.exe
```

## Desarrollo y estructura

En Windows instala Rust estable con el toolchain MSVC y las herramientas de compilación de Visual Studio (C++ y Windows SDK). Cargo descarga las dependencias públicas indicadas en `Cargo.lock`.

- `src/main.rs`: entrada de la aplicación gráfica.
- `src/lib.rs`: tipos comunes y módulos de la biblioteca local.
- `src/gui.rs`: interfaz; `src/engine.rs`: cola y transferencias.
- `src/transport.rs`: HTTP; `src/state.rs`: SQLite e integridad.
- `src/sites/`: adaptadores de los sitios incluidos en la copia del motor.
- `tests/`: pruebas con datos sintéticos y servidores locales.

```powershell
cargo test --locked
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
powershell -NoProfile -File tools/verify-standalone.ps1
```

El único binario de este paquete es la aplicación gráfica. `abrir.cmd` abre el ejecutable situado en la raíz. Para actualizarlo, cierra la aplicación y copia el archivo compilado desde `target/release/` a la raíz. Las fuentes no incluyen los ejecutables ni `downloads/`, que contiene los medios y su seguimiento privado.

## Licencia y uso responsable

El código propio se distribuye bajo la [licencia MIT](LICENSE). El propósito educativo y las responsabilidades de uso se describen en [USAGE_NOTICE.md](USAGE_NOTICE.md). Esta licencia no concede derechos sobre contenido ajeno.
