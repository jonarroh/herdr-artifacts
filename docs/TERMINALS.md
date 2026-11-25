# Terminales e imágenes

## Cómo se elige el modo

Al arrancar, tras entrar en la pantalla alternativa y antes de leer teclas, el visor consulta al terminal (mediante `ratatui-image`) qué protocolo gráfico soporta y el tamaño de celda en píxeles.

1. Si `HERDR_ARTIFACTS_IMAGE_PROTOCOL` está definida, se usa ese modo.
2. Si el terminal responde a la consulta, se usa lo que anuncia: Kitty, Sixel o iTerm2.
3. Si no responde, se usa el fallback Unicode de medios bloques.

El modo activo se ve en la cabecera (`img halfblocks`, `img kitty`…) y en `herdr-artifacts doctor`.

## Modos

| Modo | Calidad | Requisito |
| --- | --- | --- |
| `kitty` | Píxeles reales | Kitty graphics protocol |
| `sixel` | Píxeles reales, paleta reducida | Soporte Sixel |
| `iterm2` | Píxeles reales | Protocolo de imágenes en línea de iTerm2 |
| `halfblocks` | 1 × 2 "píxeles" por celda, color de 24 bits | Cualquier terminal con Unicode y color |

El fallback usa `▀` y `▄` con colores de primer plano y fondo. La imagen se escala al área disponible manteniendo la proporción.

## Dentro de Herdr

Herdr es un multiplexor: el plugin escribe en un pane virtual y Herdr redibuja en el terminal real. Eso determina qué llega.

- Herdr reenvía el **protocolo gráfico de Kitty** cuando el terminal exterior lo soporta (opción `terminal.kitty_graphics`, activada por defecto).
- La documentación de Herdr no menciona Sixel ni el protocolo de iTerm2 en panes. No cuentes con ellos dentro de Herdr.
- Herdr retiró su API de imágenes por socket y no ofrece sustituto, así que la única vía es la salida del pane.

| Terminal exterior | Dentro de Herdr |
| --- | --- |
| Kitty, Ghostty | Imágenes reales por protocolo Kitty (esperado; no verificado en el desarrollo) |
| WezTerm | Fallback Unicode: `ratatui-image` descarta Kitty y Sixel en WezTerm por errores conocidos y, dentro de Herdr, `TERM_PROGRAM` vale `herdr`, por lo que tampoco se detecta iTerm2 |
| Windows Terminal | Fallback Unicode. Soporta Sixel desde la 1.22, pero Herdr no lo reenvía. **Verificado** |
| iTerm2, Terminal.app, Alacritty | Fallback Unicode |

## Windows

En Windows la consulta de capacidades pasa por ConPTY, que no siempre entrega las respuestas del terminal al proceso. Cuando no llegan, el visor cae al fallback sin bloquearse. El tamaño de celda se asume en 10 × 20 píxeles, suficiente para medios bloques porque ahí no se dibujan píxeles reales.

## Forzar un modo

```sh
HERDR_ARTIFACTS_IMAGE_PROTOCOL=kitty herdr-artifacts tui
```

Para un pane abierto desde Herdr:

```sh
herdr plugin pane open --plugin local.herdr-artifacts --entrypoint dashboard \
  --env HERDR_ARTIFACTS_IMAGE_PROTOCOL=halfblocks
```

Forzar un protocolo que el terminal no soporta produce basura en pantalla o nada. Para volver al modo seguro usa `halfblocks`.

## Códigos QR

Los QR no dependen del modo de imagen: siempre se dibujan con bloques Unicode, en negro sobre blanco sea cual sea el tema, para que la cámara los lea.

| Presentación | Tamaño por módulo | Cuándo |
| --- | --- | --- |
| Grande | 2 columnas × 1 fila | Si cabe |
| Compacta | 1 columna × media fila | Si no cabe la grande |

Un QR de una URL corta (versión 2, 25 módulos) necesita 29 × 15 celdas con borde normal y 27 × 14 con borde reducido. Si no cabe ni así, el visor lo indica en lugar de dibujar algo ilegible; `Enter` amplía el artifact a toda el área.

Para que el QR se escanee bien hace falta una fuente con bloques que rellenen la celda completa y un interlineado sin separación. Con interlineado mayor que 1 aparecen franjas entre filas y algunos lectores fallan.

## Glifos

La interfaz usa caracteres de una celda: bordes de caja, bloques (`▀▄█▏▎▍▌▋▊▉▁▂▃▅▆▇`), e iconos geométricos (`◩ ▦ ● ¶ ▤ ▟ ✔ ▲ ✖ ℹ ▶`). No usa emoji, cuyo ancho varía entre terminales. Una fuente sin alguno de estos glifos mostrará un sustituto, sin afectar al funcionamiento.

## Color

Se emiten colores ANSI con nombre para la interfaz y RGB de 24 bits para las imágenes en medios bloques. En un terminal sin truecolor las imágenes se aproximan a la paleta disponible.

## Portapapeles

| Sistema | Método |
| --- | --- |
| Windows | API del portapapeles de Win32 |
| macOS | `pbcopy` |
| Linux | `wl-copy`, `xclip` o `xsel`, el primero disponible |
| Respaldo | Secuencia OSC 52, que pide al propio terminal fijar el portapapeles |

OSC 52 es lo que funciona por SSH, porque actúa en la máquina local. Depende de que el terminal y Herdr la acepten, y admite hasta 100 000 bytes. Con `HERDR_ARTIFACTS_CLIPBOARD=osc52` se fuerza su uso.

## Diagnóstico

```sh
herdr-artifacts doctor
```

Ejecutado dentro del pane muestra el protocolo detectado, si el terminal respondió a la consulta, el tamaño de celda y las variables relevantes (`TERM`, `TERM_PROGRAM`, `COLORTERM`).
