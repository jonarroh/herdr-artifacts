# Herdr Artifacts

Plugin para [Herdr](https://herdr.dev) que da a los agentes una superficie visual en una **ventana nativa**: códigos QR, imágenes, mensajes, Markdown, tablas, gráficas y código, con acciones interactivas cuya respuesta vuelve al agente.

La ventana es del sistema operativo (egui sobre OpenGL), flota encima de la terminal y pide las confirmaciones con el **diálogo nativo** del sistema. No hay WebView, navegador, servidor web ni runtime de JavaScript. El visor dentro de la terminal (Ratatui) sigue disponible si lo pides.

Así se ve el visor de terminal opcional:

```text
 HERDR ARTIFACTS   project my-project  agent claude  ● listening  8 artifacts  img halfblocks
┌ # snippet.rs ──────────────────── [c] Copy  ┐┏ ● Deploy to production? ━━━━━━━━ [c] Copy  ┓
│  │ rs                                       │┃El agente necesita tu aprobación             ┃
│1 │ /// Suma dos números.                    │┃                                            ┃
│2 │ pub fn add(a: i32, b: i32) -> i32 {      │┃▲ WARNING                                   ┃
│3 │     a + b                                │┃                                            ┃
│4 │ }                                        │┃La versión 1.4.2 pasó todas las pruebas.    ┃
│                                             │┃ 1:Aprobar   2:Rechazar   3:Ver logs        ┃
└──────────────────────── code · claude · now ┘┗━━━━━━━━━━ message · claude · pending · now ┛
┌ Artifact History (8) ─────────────────────────────────────────────────────────────────────┐
│   7. # snippet.rs                   code     claude       now                             │
│▶  8. ● Deploy to production?        message  claude       now  awaiting reply             │
└───────────────────────────────────────────────────────────────────────────────────────────┘
 [Tab] Navigate  [Enter] Expand  [c] Copy  [a/1-9] Actions  [d] Dismiss  [?] Help  [q] Close
```

## Requisitos

- Rust 1.95 o posterior (probado con 1.97.0).
- Herdr 0.9.4 o posterior (probado con `0.9.4-preview.2026-10-14`).
- OpenGL 2.0 o posterior para la ventana nativa.
- Windows, Linux o macOS. Desarrollado y verificado en Windows 11; en Linux y macOS compila el mismo código, pero no se ha ejecutado allí (ver [limitaciones](#limitaciones-conocidas)).

## Compilar e instalar

```sh
git clone https://github.com/jonarroh/herdr-artifacts.git
cd herdr-artifacts
cargo build --release          # binario único en target/release/
cargo test
```

En Windows sin las C++ Build Tools de Visual Studio sirve la toolchain GNU con MinGW-w64 en el `PATH`: `cargo +stable-x86_64-pc-windows-gnu build --release` (ejecútalo desde PowerShell o cmd).

Para que los agentes encuentren el CLI desde cualquier pane, ponlo en el `PATH`:

```sh
cargo install --path .         # instala herdr-artifacts en ~/.cargo/bin
```

### Registrar el plugin en Herdr

```sh
herdr plugin link /ruta/a/herdr-artifacts
```

`herdr plugin link` no compila: ejecuta antes `cargo build --release`. El manifiesto (`herdr-plugin.toml`) apunta al binario de `target/release/`.

Para retirarlo: `herdr plugin unlink local.herdr-artifacts`.

### Abrir la ventana

```sh
herdr-artifacts open
```

Abre la ventana nativa, o trae al frente la que ya está abierta. La ventana sigue viva aunque cierres el pane que la abrió, y se cierra con `q`, `Esc` o la X de la barra de título.

Un agente puede hacer que aparezca al publicar. Si ya estaba abierta, viene al frente mostrando ese artifact:

```sh
herdr-artifacts publish qr --data "exp://192.168.1.100:8081" --show
```

Sin `--show`, una ventana abierta muestra el artifact nuevo y solo parpadea en la barra de tareas, sin robar el foco.

Si prefieres el visor dentro de la terminal, `open --placement split|tab|zoomed|overlay` abre un pane de Herdr (y `herdr-artifacts tui` lo ejecuta en la terminal actual). Solo puede haber un visor a la vez recibiendo publicaciones: cierra el de terminal antes de usar la ventana.

Con un atajo de teclado, añadiendo a `config.toml` de Herdr (usa `open-windows` en Windows):

```toml
[[keys.command]]
key = "prefix+a"
type = "plugin_action"
command = "local.herdr-artifacts.open"
description = "Abrir Herdr Artifacts"
```

Después: `herdr server reload-config`.

## Publicar artifacts

El CLI es la interfaz para cualquier agente; no depende de uno concreto. Cada comando imprime una línea JSON con el resultado.

```sh
herdr-artifacts publish image ./qr.png --title "Expo QR"

herdr-artifacts publish message \
  --title "Build completed" \
  --body "La compilación terminó correctamente" --level success

herdr-artifacts publish markdown ./report.md --title "Architecture Report"

herdr-artifacts publish qr --data "exp://192.168.1.100:8081"

herdr-artifacts publish code ./src/main.rs
herdr-artifacts publish table ./build.csv            # .csv o .json
herdr-artifacts publish chart --point build=38.4 --point test=12.9 --unit s
herdr-artifacts publish json ./artifact.json         # contrato JSON completo
```

```json
{"delivered":"viewer","id":"art-3f9a12bc","ok":true,"revision":1}
```

`delivered` indica el camino: `viewer` (entregado por IPC al panel abierto) o `store` (el panel estaba cerrado; se guardó en disco y aparecerá al abrirlo).

Opciones comunes: `--id`, `--title`, `--description`, `--project`, `--agent`, `--status`, `--action`, `--show`.

### Actualizar y eliminar

```sh
herdr-artifacts publish message --id build --title "Build" --body "Compilando…" --status running
herdr-artifacts update build --body "Terminó en 38 s" --level success --status done
herdr-artifacts remove build
```

Publicar con un `--id` que ya existe reemplaza ese artifact conservando su fecha de creación.

### Acciones y respuestas

```sh
herdr-artifacts publish message --id deploy \
  --title "¿Desplegar a producción?" --body "La versión 1.4.2 está lista." \
  --action "approve:Aprobar" \
  --action "reject!:Rechazar" \
  --action "logs:Ver logs:open_url:http://localhost:8081/logs"

herdr-artifacts wait deploy --timeout 300
```

`wait` se bloquea hasta que el usuario elige una acción e imprime el evento:

```json
{"event_id":"evt-e165fedf8a81","type":"action_invoked","artifact_id":"deploy","revision":1,"action_id":"approve","action_type":"respond","agent_id":"claude","timestamp":"2026-11-25T04:23:05Z"}
```

Sale con código `0` al recibir respuesta y `2` si vence el tiempo. `herdr-artifacts events --follow` transmite todos los eventos.

Formato de `--action`: `id:etiqueta[:tipo[:url]]`. Un `!` tras el id exige confirmación. Tipos: `respond` (por defecto), `open_url`, `dismiss`.

### Otros comandos

| Comando | Qué hace |
| --- | --- |
| `herdr-artifacts` / `window` | Ejecuta la ventana nativa en este proceso |
| `tui` | Visor dentro de la terminal |
| `list [--json]` | Lista los artifacts guardados |
| `show <id>` | Imprime un artifact completo en JSON |
| `events [--follow] [--artifact ID]` | Eventos del usuario |
| `doctor` | Rutas, límites, estado del IPC y capacidades del terminal |
| `open [--placement …]` | Abre la ventana nativa (o un pane de Herdr) |

## Ventana nativa

La ventana **cambia de tamaño según el artifact** (un aviso corto abre una ventana pequeña; código, tablas, imágenes o informes, una más grande) y sigue el tema claro u oscuro del sistema. Puedes redimensionarla a mano: solo vuelve a ajustarse al pasar a otro artifact o si su contenido cambia mucho.

Las acciones son botones al pie; la primera respuesta aparece resaltada. Con más de un artifact, el botón `☰` (o `H`) abre el historial; se pone en color de aviso si hay otros esperando respuesta. **Delete**, `open_url`, `dismiss` y las acciones con `!` abren un diálogo Sí/No del sistema. Si el agente cambia o elimina el artifact mientras el diálogo está abierto, la respuesta se ignora.

| Tecla | Acción |
| --- | --- |
| `↓` `j` / `↑` `k` | Artifact siguiente / anterior |
| `1`–`9` | Ejecutar una acción |
| `c` | Copiar el texto |
| `h` | Mostrar u ocultar el historial |
| `d` / `Supr` | Borrar, con confirmación |
| `q` / `Esc` | Cerrar la ventana |

## Teclado del visor de terminal

| Tecla | Acción |
| --- | --- |
| `Tab` `↓` `j` / `Shift+Tab` `↑` `k` | Artifact siguiente / anterior |
| `g` / `G` | Primero / último |
| `Enter` | Ampliar o reducir el artifact seleccionado |
| `PgUp` / `PgDn` | Desplazar contenido largo |
| `c` o `y` | Copiar el texto al portapapeles (botón `[c] Copy`) |
| `a`, luego `←` `→` y `Enter` | Elegir y ejecutar una acción |
| `1`–`9` | Ejecutar una acción directamente |
| `d` | Descartar (borrar) el artifact, con confirmación |
| `Esc` | Volver atrás; en el nivel superior, cerrar |
| `?` | Ayuda |
| `q` / `Ctrl+C` | Cerrar |

El botón **Copy** aparece en los artifacts con contenido textual: `message`, `markdown` y `code` copian su texto original (sin numeración ni estilos), `qr` copia el dato codificado y `table` copia las filas separadas por tabuladores. Las imágenes y gráficas no lo muestran.

## Configuración

Todo el estado vive en un único directorio:

| Sistema | Ruta por defecto |
| --- | --- |
| Windows | `%LOCALAPPDATA%\herdr-artifacts` |
| Linux / macOS | `$XDG_STATE_HOME/herdr-artifacts` o `~/.local/state/herdr-artifacts` |

Variables de entorno:

| Variable | Efecto |
| --- | --- |
| `HERDR_ARTIFACTS_HOME` | Cambia el directorio de estado |
| `HERDR_ARTIFACTS_PROJECT` | Proyecto por defecto al publicar (si no, el nombre del directorio actual) |
| `HERDR_ARTIFACTS_AGENT` | Agente por defecto al publicar |
| `HERDR_ARTIFACTS_IMAGE_PROTOCOL` | Fuerza `halfblocks`, `kitty`, `sixel` o `iterm2` (por defecto, detección automática) |
| `HERDR_ARTIFACTS_CLIPBOARD` | Fuerza `native` u `osc52` para copiar |

Límites, opcionales, en `<home>/config.json` (se muestran los valores por defecto):

```json
{
  "max_file_bytes": 10485760,
  "max_payload_bytes": 1048576,
  "max_image_dimension": 4096,
  "image_cache_entries": 4,
  "history_limit": 500,
  "max_rows": 2000
}
```

## Demo reproducible

Con el panel abierto:

```sh
sh examples/demo.sh          # Linux / macOS
pwsh examples/demo.ps1       # Windows
```

Publica un artifact de cada tipo usando los fixtures de `tests/fixtures/` y espera tu respuesta a una solicitud de aprobación.

## Documentación

- [Arquitectura](docs/ARCHITECTURE.md)
- [Protocolo JSON](docs/PROTOCOL.md)
- [Terminales e imágenes](docs/TERMINALS.md)
- [Consumo de recursos](docs/PERFORMANCE.md)

## Limitaciones conocidas

- **Ventana en sesiones remotas**: por SSH o sin entorno gráfico la ventana no puede abrirse (o se abre en la máquina remota); usa el visor de terminal. Si falla, el motivo queda en `<home>/window.log`.
- **Foco**: Windows puede impedir que una ventana lanzada en segundo plano tome el foco del teclado; queda encima, pero a veces hay que hacer clic en ella.
- **Markdown en la ventana**: sin imágenes embebidas y con las tablas alineadas de forma simple; los enlaces no son clicables a propósito.

Del visor de terminal:

- **Imágenes reales dentro de Herdr**: Herdr solo reenvía el protocolo gráfico de Kitty, y solo si el terminal exterior lo soporta. En Windows Terminal (aunque soporta Sixel) el plugin usa el fallback Unicode. Detalle en [docs/TERMINALS.md](docs/TERMINALS.md).
- **Protocolos de imagen real no verificados visualmente**: el desarrollo se hizo en Windows Terminal, que cae al fallback. Kitty, Sixel e iTerm2 están cubiertos por una prueba de codificación, no por una comprobación en pantalla.
- **Linux y macOS**: el código es portable y las rutas específicas (socket Unix, `pbcopy`, `xdg-open`) están implementadas, pero solo se ha ejecutado en Windows.
- **Botón Copy solo por teclado**: no se captura el ratón, para no interferir con la selección de texto de Herdr.
- **Sin resaltado de sintaxis** en `code` (fuera del alcance del MVP).
- **Un visor principal por directorio de estado**: un segundo panel funciona en modo secundario (lee del disco cada segundo, sin recibir IPC).

## Licencia

[MIT](LICENSE)
