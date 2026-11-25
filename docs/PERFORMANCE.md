# Consumo de recursos

## Medición

Entorno: Windows 11, Herdr 0.9.4-preview dentro de Windows Terminal, binario `release`, visor abierto en un pane de Herdr con 8 artifacts de todos los tipos (incluida una imagen). Medido con `scripts/measure.ps1 -IdleSeconds 30 -Burst 50` el 2026-11-24.

| Fase | Duración | CPU (s) | CPU (% de un núcleo) | Working set | Memoria privada | Hilos |
| --- | --- | --- | --- | --- | --- | --- |
| Reposo | 30,0 s | 0,05 | 0,16 % | 8,0 MB | 1,8 MB | 2 |
| Ráfaga: 50 publicaciones por IPC | 6,9 s | 0,50 | 7,30 % | 8,2 MB | 1,9 MB | 3 |
| Limpieza: 50 borrados | 5,5 s | 0,30 | 5,43 % | 8,3 MB | 1,9 MB | 3 |
| Reposo posterior | 10,0 s | 0,03 | 0,31 % | 8,3 MB | 1,9 MB | 3 |

Pico de working set: 9,5 MB. Tamaño del binario: 2,09 MB.

Lectura de los datos:

- En reposo el visor consume unos **8 MB de RAM** y **0,16 % de un núcleo**. Esa CPU es el `poll` de eventos cada 100 ms.
- La ráfaga cuesta unos 10 ms de CPU por publicación, que incluyen validar, escribir dos archivos y redibujar la pantalla.
- La memoria no crece con la ráfaga: los 50 artifacts añadidos solo suman sus metadatos.
- Los dos hilos en reposo son la UI y la escucha del IPC. El tercero que aparece tras la ráfaga no lo crea el plugin de forma permanente (los hilos de conexión terminan al responder); no se investigó su origen y se atribuye, sin confirmarlo, al pool de hilos del sistema.

La medición cubre una sola máquina y una sola ejecución. No se midió con imágenes grandes ni con protocolos de imagen real, que guardan en caché una codificación mayor que la de medios bloques.

## Repetir la medición

Con el panel abierto en Herdr:

```sh
pwsh scripts/measure.ps1 -IdleSeconds 30 -Burst 50     # Windows
sh scripts/measure.sh 30 50                            # Linux / macOS
```

El script localiza el proceso del visor, lo muestrea en reposo, publica y borra una ráfaga de mensajes por IPC y vuelve a muestrear.

## Por qué consume poco

| Técnica | Dónde |
| --- | --- |
| Dibujar solo cuando algo cambia | `App::event_loop`: una bandera `dirty` que activan teclas, redimensionados y peticiones IPC |
| Carga bajo demanda | En memoria solo están los metadatos; el contenido se lee del disco para los artifacts visibles (dos como máximo) y se libera al salir de pantalla |
| Imágenes perezosas | Se decodifican al mostrarse; se reducen antes de codificar; el bitmap se descarta y solo queda la codificación final |
| Caché acotada | LRU de 4 imágenes (`image_cache_entries`), vaciada al redimensionar |
| Tablas largas | Solo se construyen las filas visibles |
| Sin runtime asíncrono | Un hilo bloqueado en `accept`, hilos efímeros por conexión, sin Tokio |
| Sin procesos auxiliares | Herdr ejecuta el binario directamente; el portapapeles en Windows usa la API nativa |
| Historial acotado | `history_limit` (500) elimina los más antiguos; el log de eventos se recorta a 2 MB |
| Límites de entrada | Tamaño de archivo, de contenido, dimensiones de imagen y memoria del decodificador |
| Binario pequeño | `opt-level = "s"`, LTO, un solo `codegen-unit`, `panic = "abort"`, `strip` |

## Costes conocidos

- **Arranque**: lee todos los `meta/<id>.json`. Con el máximo de 500 son 500 archivos de unos 200 bytes.
- **Latencia de IPC**: el visor atiende las peticiones entre dos `poll`, así que una publicación puede tardar hasta 100 ms en responderse.
- **Visor secundario**: un segundo panel sobre el mismo almacén consulta la fecha del directorio una vez por segundo.
- **Imágenes reales**: la codificación Sixel o Kitty de una imagen a pantalla completa ocupa bastante más que la de medios bloques. El tope lo fija la caché de 4 entradas.
