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
| Protocolo compartido | Mensajes M1 y encuadre acotado implementados | Mensajes versionados de control, vídeo, entrada y telemetría |
| Runtime compartido | Negociación, sesiones, reconexión, telemetría, ritmo y loopback implementados | Runtime común para hosts y pantallas |
| Host de escritorio | Aplicación Tauri 2 con loopback y diagnósticos ejecutable | Una interfaz con servicios nativos por plataforma |
| Host macOS | Permisos/pantallas, prueba IOSurface real con ScreenCaptureKit y paquete local implementados | Flujo continuo con VideoToolbox, pantalla virtual nativa y aplicación notarizada |
| Host Windows | Captura, H.264 por GPU y entrada verificados; controlador IddCx, servicio LocalSystem y cliente IPC acotado compilados y validados | Instalación confiable, selección automática, recuperación en sistema limpio y firma de producción |
| Host Linux | Solo arquitectura | Integración compatible con Wayland, X11 y DRM |
| Pantalla Android | Solo arquitectura | Receptor Kotlin nativo con decodificación por hardware |
| Pantalla iOS/iPadOS | Solo arquitectura | Receptor Swift nativo con decodificación por hardware |
| Transporte USB | AOA, transporte dúplex acotado, H.264 real y entrada implementados; falta la prueba física completa con Android | Enlace directo, autenticado y reconectable |
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

## Ejecutar la base de escritorio actual

Instala Rust 1.97.1, Node.js LTS, pnpm 10.26.0 y los
[requisitos de Tauri](https://v2.tauri.app/start/prerequisites/) para tu sistema.

```bash
pnpm install --frozen-lockfile
pnpm check
pnpm dev:desktop
```

La aplicación negocia una sesión real y mueve fotogramas sintéticos por el
transporte acotado mientras muestra telemetría. Es una base verificable, no un
monitor extendido listo para usar. Consulta la [configuración](./docs/development.md)
y el [traspaso de plataformas](./docs/platform-handoff.md).

## Licencia

El código se publica bajo la [licencia MIT](./LICENSE). El nombre y el logotipo identifican al proyecto; su redistribución no debe insinuar el respaldo oficial de LadoFlow.
