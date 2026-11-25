# Arquitectura

## Visión general

Un único binario cumple dos papeles:

- **Visor** (`herdr-artifacts tui`): la TUI que Herdr ejecuta dentro de un pane.
- **CLI** (`herdr-artifacts publish …`): lo que invocan los agentes desde cualquier pane.

```text
 agente (cualquier pane)                         pane del plugin
┌──────────────────────┐   1 línea JSON        ┌───────────────────────────────┐
│ herdr-artifacts      │ ───────────────────▶  │ hilo IPC (accept bloqueante)  │
│   publish / update   │   socket local        │        │ canal mpsc           │
│                      │ ◀───────────────────  │        ▼                      │
│                      │   1 línea JSON        │ hilo UI: valida, guarda,      │
└──────────┬───────────┘                       │          redibuja             │
           │ si no hay visor                   └───────┬───────────────────────┘
           ▼                                           │
   ┌────────────────────────────── disco ──────────────▼─────────┐
   │ meta/<id>.json  data/<id>.json  files/<img>  events.jsonl   │
   └──────────────────────────────▲──────────────────────────────┘
                                  │ wait / events (lectura incremental)
                          agente esperando respuesta
```

El disco es la fuente de verdad. El IPC solo añade inmediatez: si el visor está cerrado, el CLI aplica la misma operación directamente sobre el almacén y el artifact aparece al abrir el panel.

## Módulos

| Módulo | Responsabilidad | E/S |
| --- | --- | --- |
| `protocol/artifact.rs` | Modelos, contenido tipado por tipo, validación y saneado | Ninguna |
| `protocol/events.rs` | Mensajes de IPC y eventos de usuario | Ninguna |
| `storage/repository.rs` | Persistencia, historial, ingesta de imágenes, log de eventos | Disco |
| `transport/mod.rs` | `handle()`: aplica una petición al almacén | — |
| `transport/ipc.rs` | Socket local, token, hilo de escucha, cliente | Socket |
| `renderer/*` | Un renderer por tipo; caché de imágenes | Lee `files/` |
| `terminal/capabilities.rs` | Detección del protocolo gráfico | Consulta al terminal |
| `interaction/mod.rs` | Máquina de estados de navegación (tecla → comando) | Ninguna |
| `interaction/actions.rs` | Ejecuta acciones y genera eventos | Disco, abrir URL |
| `interaction/clipboard.rs` | Copiar al portapapeles | Portapapeles |
| `app.rs` | Layout, dibujo y bucle de eventos | Terminal |
| `cli.rs` | Argumentos y comandos para agentes | — |
| `config.rs`, `time.rs`, `error.rs` | Rutas y límites, RFC 3339, errores con código | — |

Las dependencias van en un solo sentido: `protocol` no conoce a nadie; `storage` y `renderer` dependen de `protocol`; `app` y `cli` orquestan.

## Decisiones de diseño

**Archivos en lugar de SQLite.** El historial son como mucho unos cientos de documentos pequeños. Con un archivo por artifact se evita compilar C, cada escritura es atómica (archivo temporal + `rename`) y no hay un índice compartido que dos procesos puedan corromper.

**Metadatos separados del contenido.** `meta/<id>.json` guarda solo lo que necesita el historial (título, tipo, agente, estado). El visor mantiene esa lista en memoria y lee `data/<id>.json` únicamente para los artifacts en pantalla, que son dos como máximo.

**Sin Tokio.** El visor no tiene concurrencia real: un hilo bloqueado en `accept` (sin CPU en reposo) y hilos efímeros por conexión que solo analizan y autentican. Las peticiones llegan por canal al hilo de UI, único dueño del almacén, así que no hay locks.

**Una sola ruta de validación.** `transport::handle` es el único punto donde una petición toca el almacén, tanto si llega por IPC como si el CLI escribe en disco. Ambos caminos aplican las mismas reglas.

**Interacción como máquina de estados pura.** `UiState::handle_key` recibe una tecla y devuelve un `Command` sin hacer E/S. Toda la navegación se prueba sin terminal.

**Hilo por conexión con tope.** Los named pipes de Windows no admiten timeouts de E/S. Para que un cliente que se conecta y calla no bloquee la escucha, cada conexión tiene su hilo efímero, con un máximo de 8 simultáneas.

## Añadir un tipo de artifact

El sobre (`Artifact`) trata `type` como texto y `content` como JSON opaco, así que almacenamiento, transporte e historial no cambian. Hacen falta cuatro pasos:

1. Añadir la variante a `Content` y su rama en `Content::parse` (`protocol/artifact.rs`), con su validación.
2. Añadir el nombre a `ARTIFACT_TYPES`.
3. Crear `renderer/<tipo>.rs` y su rama en `renderer::render`, más el icono en `type_icon`.
4. Opcional: una rama en `cli::build_input` para publicarlo con argumentos, y en `Content::copy_text` si es copiable. Sin esto ya se puede publicar con `publish json`.

## Seguridad

Todo lo que publica un agente se trata como no confiable.

| Riesgo | Medida |
| --- | --- |
| Path traversal | Un artifact `image` solo puede referenciar un nombre de archivo simple dentro de `files/`. El CLI copia ahí la imagen; el visor nunca abre una ruta del agente. Los ids se limitan a `[A-Za-z0-9._-]`. |
| Archivos enormes | Límite de bytes antes de copiar, límite de dimensiones leyendo solo la cabecera y tope de memoria del decodificador (128 MB). |
| Inyección de escapes de terminal | Se eliminan los caracteres de control de todo texto al validar. |
| HTML / JavaScript | Markdown muestra el HTML como texto literal. No existe ningún intérprete. |
| Ejecución de comandos | No hay ningún tipo de acción que ejecute comandos. Los tipos desconocidos se rechazan al publicar. |
| URLs | Solo `http://` y `https://`, sin espacios, comillas ni credenciales embebidas. Se validan al publicar y otra vez antes de abrir. La URL se pasa como argumento único, sin shell, y siempre pide confirmación. |
| Acceso al IPC | Unix: socket dentro de un directorio `0700`. Windows: cada petición debe llevar un token de 256 bits guardado en el perfil del usuario; se compara en tiempo constante. |
| JSON malformado o gigante | Esquema validado con `serde`; las líneas que superan el límite se rechazan. |
| Acciones sensibles | `open_url`, `dismiss` y cualquier acción con `confirm: true` requieren confirmación. |

Límite conocido: los procesos del mismo usuario se consideran autorizados. Cualquiera de ellos puede leer el token y publicar, igual que puede ejecutar el CLI.

## Pruebas

```sh
cargo test
```

| Suite | Cubre |
| --- | --- |
| Unitarias (`src/`) | Serialización, validación, URLs, saneado, RFC 3339, cada renderer, detección de protocolo, máquina de estados, autenticación |
| `tests/storage.rs` | Persistencia tras reabrir, orden, actualización, límites, traversal, eventos |
| `tests/ipc.rs` | Ida y vuelta real por socket / named pipe, token incorrecto, aislamiento |
| `tests/app.rs` | El visor sobre un backend de prueba: layout, navegación, acciones, copiar, actualizaciones en vivo |
| `tests/cli.rs` | El binario real de extremo a extremo, con y sin visor |

Los fixtures están en `tests/fixtures/`.
