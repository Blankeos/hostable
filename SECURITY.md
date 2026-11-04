# Security

`hostabled` runs as root on a machine that is usually exposed to the internet, so security reports are taken seriously.

## Reporting a vulnerability

Please do **not** open a public issue. Use GitHub's private vulnerability reporting on this repository (Security tab, "Report a vulnerability"), or email carloantonioct@gmail.com.

Include the version (`hostable --version`), what you did, and what happened. You will get a reply within a few days.

## Scope

In scope: the daemon API, the installer, the web admin, input validation, file permissions and anything that lets an unauthenticated or unprivileged user act as root.

Out of scope: the games themselves (Minecraft, Palworld, ARK), and servers you deliberately configured without authentication such as `online-mode=false`.
