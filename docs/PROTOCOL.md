# Protocolo JSON

Versión del protocolo: `1`.

## 1. Artifact

```json
{
  "id": "artifact-001",
  "type": "image",
  "title": "Expo Development QR",
  "description": "Escanea para abrir la aplicación",
  "created_at": "2026-11-24T20:00:00Z",
  "updated_at": "2026-11-24T20:05:12.431Z",
  "project_id": "my-project",
  "agent_id": "claude",
  "status": "ready",
  "revision": 2,
  "response": "open",
  "content": { "source": "img-9c1e52aa0b7d4f10.png" },
  "actions": [
    {
      "id": "open",
      "label": "Abrir",
      "type": "open_url",
      "payload": { "url": "http://localhost:8081" }
    }
  ]
}
```

| Campo | Tipo | Al publicar | Reglas |
| --- | --- | --- | --- |
| `id` | string | Opcional (se genera `art-xxxxxxxx`) | 1–64 caracteres de `[A-Za-z0-9._-]`, sin empezar por `.` |
| `type` | string | Obligatorio | Uno de los tipos de la sección 2 |
| `title` | string | Opcional (por defecto, el tipo o el nombre del archivo) | 1–200 caracteres, una línea |
| `description` | string | Opcional | Hasta 1000 caracteres |
| `created_at` | RFC 3339 | Opcional (ahora) | Se normaliza a UTC; precisión de milisegundos |
| `project_id`, `agent_id`, `status` | string | Opcionales | Hasta 64 caracteres; `status` es texto libre |
| `content` | objeto | Obligatorio | Según `type`; tamaño máximo `max_payload_bytes` |
| `actions` | array | Opcional | Hasta 9; ver sección 3 |
| `updated_at` | RFC 3339 | Lo gestiona el plugin | Presente tras la primera actualización |
| `revision` | entero | Lo gestiona el plugin | Empieza en 1 y sube con cada cambio |
| `response` | string | Lo gestiona el plugin | Id de la última acción elegida; se borra al actualizar |

Los caracteres de control se eliminan de todos los textos. Los campos desconocidos se ignoran, de modo que un cliente más nuevo puede enviar campos que esta versión no conoce.

## 2. Tipos de contenido

### `image`

```json
{ "source": "img-9c1e52aa0b7d4f10.png" }
```

`source` es el nombre de un archivo dentro del directorio `files/` del almacén. El CLI se encarga: `publish image ./qr.png` copia el archivo y escribe el nombre. Con `publish json`, `source` puede ser una ruta local y el CLI la ingiere igual. Formatos: PNG, JPEG, GIF, WebP y BMP.

### `qr`

```json
{ "data": "exp://192.168.1.100:8081" }
```

De 1 a 2000 bytes, sin caracteres de control.

### `message`

```json
{ "body": "Build completed\n12 files generated", "level": "success" }
```

`level`: `info` (por defecto), `success`, `warning` o `error`.

### `markdown`

```json
{ "text": "# Título\n\nTexto con **negrita**." }
```

CommonMark con tablas, tachado y listas de tareas. El HTML se muestra como texto.

### `code`

```json
{ "text": "fn main() {}", "language": "rust" }
```

`language` es opcional y solo informativo.

### `table`

```json
{ "columns": ["Step", "Duration"], "rows": [["compile", "38.4s"], ["test", 12.9]] }
```

De 1 a 32 columnas. Cada fila debe tener tantas celdas como columnas. Las celdas pueden ser texto, números, booleanos o `null`.

### `chart`

```json
{
  "style": "bar",
  "unit": "s",
  "points": [ { "label": "compile", "value": 38.4 }, { "label": "test", "value": 12.9 } ]
}
```

`style`: `bar` (por defecto) o `line`. `unit` es opcional. Los valores deben ser números finitos.

## 3. Acciones

```json
{ "id": "approve", "label": "Aprobar", "type": "respond", "payload": { "env": "production" }, "confirm": false }
```

| Campo | Reglas |
| --- | --- |
| `id` | Mismas reglas que el id de artifact; único dentro del artifact |
| `label` | 1–64 caracteres |
| `type` | `respond` (por defecto), `open_url` o `dismiss` |
| `payload` | JSON libre; se devuelve tal cual en el evento |
| `confirm` | Si es `true`, pide confirmación antes de ejecutar |

| Tipo | Efecto | Confirmación |
| --- | --- | --- |
| `respond` | Emite un evento `action_invoked` | Solo si `confirm` es `true` |
| `open_url` | Abre `payload.url` en el navegador y emite el evento | Siempre |
| `dismiss` | Emite `artifact_dismissed` y borra el artifact | Siempre |

`payload.url` debe ser `http://` o `https://`, de hasta 2048 caracteres, sin espacios, comillas, barras invertidas ni credenciales (`user:pass@`).

Cualquier otro tipo se rechaza al publicar. No existe una acción que ejecute comandos.

## 4. Transporte IPC

- **Linux / macOS**: socket de dominio Unix en `<home>/ipc.sock`.
- **Windows**: named pipe `\\.\pipe\herdr-artifacts-<hash del home>`.

Una conexión transporta una petición y una respuesta, cada una en una línea JSON terminada en `\n`.

### Petición

```json
{ "v": 1, "token": "<contenido de <home>/ipc.token>", "op": "publish", "artifact": { "type": "qr", "content": { "data": "exp://…" } } }
```

| `op` | Campos | Efecto |
| --- | --- | --- |
| `publish` | `artifact` | Crea el artifact o reemplaza el del mismo `id` |
| `update` | `id`, `patch` | Modifica un artifact existente |
| `remove` | `id` | Lo borra |
| `show` | `id` (opcional) | Trae la ventana al frente y selecciona ese artifact. El visor de terminal responde `invalid` |
| `ping` | — | Comprueba que el visor responde |

`patch` admite `title`, `description`, `status`, `content` y `actions`. `content` se fusiona superficialmente con el existente: las claves enviadas sustituyen a las anteriores y el resto se conserva. `actions` reemplaza la lista completa.

```json
{ "v": 1, "token": "…", "op": "update", "id": "build", "patch": { "status": "done", "content": { "level": "success" } } }
```

### Respuesta

```json
{ "status": "ok", "id": "build", "revision": 2 }
```

```json
{ "status": "error", "code": "invalid", "message": "invalid artifact: unknown artifact type 'html' (supported: image, qr, message, markdown, table, chart, code)" }
```

| `code` | Significado |
| --- | --- |
| `invalid` | El artifact, el parche o la petición no pasan la validación |
| `not_found` | No existe un artifact con ese `id` |
| `too_large` | Un archivo o el contenido supera un límite |
| `unauthorized` | Token ausente o incorrecto |
| `io` | Fallo de disco o de socket |

El CLI imprime los errores como `error[<code>]: <mensaje>` en stderr y sale con código 1.

## 5. Eventos de usuario

El visor añade una línea JSON a `<home>/events.jsonl` por cada interacción relevante.

```json
{
  "event_id": "evt-e165fedf8a81",
  "type": "action_invoked",
  "artifact_id": "deploy-approval",
  "revision": 1,
  "action_id": "approve",
  "action_type": "respond",
  "payload": { "env": "production" },
  "project_id": "my-project",
  "agent_id": "claude",
  "timestamp": "2026-11-25T04:23:05Z"
}
```

| `type` | Cuándo |
| --- | --- |
| `action_invoked` | El usuario ejecutó una acción `respond` u `open_url` |
| `artifact_dismissed` | El usuario borró el artifact, con `d` o con una acción `dismiss` |

`action_id`, `action_type` y `payload` faltan cuando el usuario borra con `d`.

### Leer eventos

```sh
herdr-artifacts wait <id> [--timeout SEGUNDOS]   # el siguiente evento de ese artifact
herdr-artifacts events [--follow] [--artifact ID]
```

`wait` devuelve el primer evento cuyo `revision` sea igual o mayor que la revisión actual del artifact. Encuentra también respuestas dadas antes de empezar a esperar, así que no hay carrera entre publicar y esperar. Al actualizar el artifact sube la revisión y las respuestas anteriores dejan de contar: una actualización equivale a una pregunta nueva.

El archivo se recorta a las últimas 1000 líneas cuando supera 2 MB.

## 6. Extensión

- **Nuevos tipos de artifact**: un valor nuevo de `type` con su `content`. El sobre no cambia.
- **Nuevas operaciones**: un valor nuevo de `op`.
- **Nuevos eventos**: un valor nuevo de `type` en el evento. Los lectores deben ignorar los que no conozcan.
- **Cambios incompatibles**: suben `v`. El visor rechaza versiones que no entiende con `invalid`.
