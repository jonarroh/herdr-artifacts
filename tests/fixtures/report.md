# Architecture Report

El servicio se divide en **tres capas** con responsabilidades claras.

## Componentes

- `api`: endpoints HTTP y validación
- `core`: reglas de negocio
  - sin dependencias de red
- `storage`: persistencia en *SQLite*

## Pendientes

1. Migrar los tests de integración
2. Medir latencia p95

- [x] Diseño aprobado
- [ ] Revisión de seguridad

```rust
fn main() {
    println!("hola");
}
```

> Nota: los tiempos son estimados.

| Capa    | Archivos | Cobertura |
|---------|----------|-----------|
| api     | 12       | 81%       |
| core    | 30       | 94%       |

---

Más detalles en [la documentación](https://herdr.dev/docs).

<script>alert("esto nunca se ejecuta")</script>
