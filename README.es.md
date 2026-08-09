<p align="center">
  <img src="./assets/brand/ladoflow-mark-256.png" width="176" alt="Logotipo de LadoFlow">
</p>

<h1 align="center">LadoFlow</h1>

<p align="center">
  <strong>Convierte la pantalla que tienes al lado en un segundo monitor fluido y privado.</strong>
</p>

<p align="center">Primero USB · siempre local · sin cuenta · código abierto</p>

<p align="center">
  <a href="./README.md">English</a> ·
  <a href="./README.es.md">Español</a> ·
  <a href="./README.zh-CN.md">简体中文</a>
</p>

> [!IMPORTANT]
> LadoFlow se encuentra en una etapa **prealfa**. Todavía no existe una versión funcional para usar como segundo monitor. Este documento separa las bases ya iniciadas de la compatibilidad prevista.

## La idea

LadoFlow pretende conectar equipos Windows, macOS y Linux con tabletas y teléfonos Android, iPad y iPhone. La primera ruta de transporte será USB; la red local llegará después de demostrar que la conexión por cable es estable y medible.

La experiencia final deberá ser sencilla: instalar el host, abrir LadoFlow en la tableta, conectar el cable y ampliar el escritorio. Sin cuenta obligatoria, sin retransmisión por la nube y sin suscripción.

## Estado real

| Área | Estado actual | Objetivo |
| --- | --- | --- |
| Protocolo compartido | Encuadre binario inicial implementado | Mensajes versionados de control, vídeo, entrada y telemetría |
| Host Windows | Solo arquitectura | Controlador de pantalla virtual firmado y aplicación host |
| Host macOS | Solo arquitectura | Adaptador nativo y aplicación notarizada |
| Host Linux | Solo arquitectura | Integración compatible con Wayland, X11 y DRM |
| Pantalla Android | Solo arquitectura | Receptor Kotlin nativo con decodificación por hardware |
| Pantalla iOS/iPadOS | Solo arquitectura | Receptor Swift nativo con decodificación por hardware |
| Transporte USB | Solo arquitectura | Enlace directo, autenticado y reconectable |
| Wi-Fi/LAN | Planificado | Conexión local mediante emparejamiento explícito |

La [hoja de ruta](./docs/roadmap.md) es la referencia para el avance verificable.

## Por qué se llama LadoFlow

**Lado** expresa la pantalla que está junto al ordenador. **Flow** expresa fluidez, ritmo y continuidad. El nombre resume la promesa: *una pantalla a tu lado que no interrumpe tu ritmo de trabajo*.

El símbolo muestra dos pantallas contiguas unidas por un recorrido continuo. La línea también insinúa una **L**: representa el transporte local de imagen y entrada. El punto coral identifica el dispositivo conectado.

## Diseño técnico

- Los controladores de pantalla, códecs, renderizado, USB e inyección de entrada serán nativos.
- Rust compartirá el protocolo, las sesiones, la negociación de capacidades y la adaptación de calidad.
- Android utilizará Kotlin y las API multimedia del sistema.
- iOS/iPadOS utilizará Swift y los marcos multimedia de Apple.
- Cada afirmación de latencia deberá estar respaldada por mediciones reproducibles.

Consulta la [arquitectura](./docs/architecture.md), el [protocolo](./docs/protocol.md) y la [guía de marca](./docs/brand.md).

## Compilar la base actual

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Licencia

El código se publica bajo la [licencia MIT](./LICENSE). El nombre y el logotipo identifican al proyecto; su redistribución no debe insinuar el respaldo oficial de LadoFlow.
