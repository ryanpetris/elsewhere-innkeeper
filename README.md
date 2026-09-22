# Elsewhere Innkeeper

A separate Rust/Axum and React/Vite application that creates and manages Elsewhere desktops in Docker. Choose Arch Linux, Debian or Ubuntu, add package names, and open a ready desktop in a new tab. The workspace lists sessions as a grid of authenticated desktop previews or as a dense list, and each session has a page of its own carrying its configuration, live logs, sharing, and every action grouped by what it changes.

## Run with Docker Compose

Compose enables HTTPS automatically. On first start, Innkeeper generates a self-signed
certificate and private key in its data volume and reuses them on subsequent starts.

```sh
docker compose up -d --build
```

Open `https://<server-hostname>:19300` and create the first Administrator with your chosen username, display name, and password. Complete setup on a trusted network before exposing the installation. Setup is available only while no users exist. Compose retains Innkeeper state in `innkeeper-data`; desktop home directories use separate Docker volumes.

The browser shows a certificate warning for the self-signed certificate. Compare its
SHA-256 fingerprint with `docker compose logs innkeeper` before accepting the exception.

Innkeeper downloads the pinned Elsewhere release package from GitHub and caches it in its data directory. It pulls a stock distribution image if needed, then installs the package and its dependencies inside each new container. Open the session and expand **Logs** to follow downloads and setup. Innkeeper does not clone or compile Elsewhere at runtime.

Sessions open at `/e/<session-uuid>/` on Innkeeper's origin. Serve that origin over HTTPS for WebCodecs and browser capture features. Outside Compose, set `INNKEEPER_TLS=1` for an automatically generated certificate, set `INNKEEPER_TLS_CERT` and `INNKEEPER_TLS_KEY` to your own PEM files, or put Innkeeper behind an HTTPS gateway. Elsewhere serves plain HTTP privately; Innkeeper proxies HTTP and WebSockets. WebRTC uses a separate encrypted UDP connection directly to each session.

## Run the Docker Hub image

The published image supports Linux amd64. Replace `<namespace>` with the Docker Hub namespace:

```sh
docker run -d --name elsewhere-innkeeper --restart unless-stopped \
  -p 19300:19300 \
  -v /var/run/docker.sock:/var/run/docker.sock \
  -v /dev/dri:/dev/dri:ro \
  -v innkeeper-data:/var/lib/elsewhere-innkeeper \
  <namespace>/elsewhere-innkeeper:latest
```

Use `:X.Y.Z` to select a specific Innkeeper release. Every release pushes both its version tag and
`latest`; `latest` points to whichever release most recently pushed that tag. The network and HTTPS
configuration below applies to the published image too.

## The web interface

Every page has an address, so a session can be linked to, reloaded, and reached with the
browser's own back and forward buttons.

| Address | Page |
| --- | --- |
| `/` | The workspace: every session you can reach, as a grid or a list |
| `/sessions/new` | Create a session |
| `/sessions/<id>` | One session: preview, configuration, runtime, logs, sharing, and its actions |
| `/sessions/<id>/settings` | Change session settings, packages, Docker options, and GPU selection |
| `/account` | Your display name and password |
| `/users` | Every account (Administrators) |
| `/users/new` | Create an account (Administrators) |
| `/users/<id>` | One account: identity, role, password reset, and deletion (Administrators) |

The grid and list choice is remembered in the browser. Innkeeper serves the same document
for every one of these addresses; the browser decides which page to draw.

## Accounts and sharing

Accounts use a unique username, display name, and password. Administrators create accounts; there is no signup page, email configuration, or email recovery. Users can change their display name and password on their own account page. Only Administrators can rename usernames, manage accounts, and change machine sharing; **Users** lists every account and opens one for editing.

| Machine access | Allowed actions |
| --- | --- |
| Viewer | See the machine and preview; connect with video, audio, and clipboard text/image reads |
| Interactive | Viewer access plus desktop input, applications, commands, files, broadcasts, microphone, and camera |
| Manager | Interactive access plus start, stop, edit, logs, upgrade, relaunch, and delete |

Creators receive an ordinary Manager assignment. Administrators can downgrade or revoke it, and always manage every machine themselves. Machines have no owner account and remain after an account is deleted. Normal users cannot supply Docker security options.

Passwords use salted Argon2id hashes. Login cookies are HttpOnly, Secure, SameSite=Strict, and scoped to `/api`. HTTPS is required for authenticated browser use. The open UI checks once a minute and on focus, visibility, and network changes; in the final two days it renews a login to seven days from renewal. An expired login requires signing in again. Password changes invalidate all that user's logins. Logging out or reaching login expiry does not revoke a desktop token.

Opening a machine reuses its user's correctly permissioned token. A permission mismatch permanently marks that credential revoked, attempts remote deletion, and creates a replacement if authorized. Startup, machine return, access changes, and periodic synchronization only retire/delete invalid user tokens. They never create replacements. Remote deletion retries in memory with exponential delays from five seconds to five minutes. Retired records remain in SQLite until Elsewhere confirms deletion, even if access is restored. Successful deletion disconnects resources using that credential; completed desktop commands and file changes are not undone. Browser tokens never grant token or server administration. Viewer has no broadcast access.

If the last Administrator loses their password, stop Innkeeper and run `elsewhere-innkeeper users list`, then `elsewhere-innkeeper users reset-password --id UUID` using the same data directory. The reset command prompts twice without echo and invalidates that account's logins. It acquires the same exclusive data-directory lock as the server. There is no unauthenticated recovery API.

## Network configuration

Innkeeper listens on port `19300`. Each session reserves a port from `19500` through `19999` until destruction, including while stopped. WebRTC publishes `0.0.0.0:P:P/udp`. Open or forward that UDP range without changing port numbers. WebRTC uses the hostname in the browser URL with the session’s assigned UDP port. That hostname must resolve to a reachable address from inside the session container; set `INNKEEPER_RTC_ADDR` only to override it. Docker rejects occupied ports; the failed session remains available for cleanup. WebSocket video remains available when UDP connectivity fails.

| Variable | Default | Purpose |
| --- | --- | --- |
| `INNKEEPER_LISTEN` | `0.0.0.0:19300` | Innkeeper HTTP or HTTPS listen address |
| `INNKEEPER_RTC_ADDR` | Browser hostname | Optional reachable IPv4 address override for WebRTC |
| `INNKEEPER_IN_DOCKER` | `0` native, `1` in image | Reach sessions through a shared Docker bridge |
| `INNKEEPER_DOCKER_CONTAINER` | Container hostname | Innkeeper container name or ID for Docker inspection |
| `INNKEEPER_DOCKER_NETWORK` | Discover one attached bridge | Select an attached bridge by name or ID when there are several |
| `INNKEEPER_TLS` | `0`; `1` in Compose | Generate and reuse a self-signed certificate for HTTPS |
| `INNKEEPER_TLS_CERT` | Empty | PEM certificate chain; enables HTTPS together with the key |
| `INNKEEPER_TLS_KEY` | Empty | PEM private key |
| `INNKEEPER_DATA_DIR` | `/var/lib/elsewhere-innkeeper` | Private account/session state and build logs |
| `INNKEEPER_ASSETS_DIR` | `/usr/share/elsewhere-innkeeper` | Session setup scripts |

In Docker mode, Innkeeper discovers its own bridge network through Docker inspection and attaches new sessions to that network. Their HTTP ports are not published. Requests use each owned container's current IP and port `19443`, so both the default bridge and custom networks work without container-name DNS. Peers on that bridge can reach session HTTP; Elsewhere authenticates its API and WebSockets. If the container has a custom hostname, set `INNKEEPER_DOCKER_CONTAINER` to its Docker name or ID. The CLI equivalents are `--in-docker`, `--docker-container NAME_OR_ID`, and `--docker-network NAME_OR_ID`.

Native Innkeeper publishes session HTTP at `127.0.0.1:P:19443/tcp` and connects through loopback. One Innkeeper controls one local Linux Docker daemon. Remote daemons, rootless Docker, and changing an installation between native and Docker modes are unsupported. Start uses the container's existing Docker configuration.

Automatic TLS stores `cert.pem` and `key.pem` in `INNKEEPER_DATA_DIR`, with mode `0600`.
The certificate covers localhost, loopback addresses, and Innkeeper's network interface
addresses at generation time. In Docker, these are container addresses, not the host's
LAN addresses. Stop Innkeeper and delete both files to generate a new pair on next start.
Explicit certificate and key paths take precedence over automatic generation; missing
or invalid explicit files fail startup. For HTTP behind an external HTTPS gateway, set
`INNKEEPER_TLS=0` and leave both explicit certificate variables unset.

Restart Innkeeper after renewing certificates. An external HTTPS gateway must connect to Innkeeper using HTTP/1.1 and forward the complete path, authorization, WebSocket upgrades and streaming bodies. Session links use the browser's origin; no public-host override is needed. URL prefixes keep Elsewhere preferences and tokens separate but do not isolate applications within the browser origin.

Login permits five failed attempts per username and 30 per transport peer IP each minute. Behind an HTTPS gateway, all users share the gateway IP budget. Forwarded address headers are not trusted.

After changing Compose configuration, rebuild and recreate Innkeeper with `docker compose up -d --build`.

Innkeeper controls Docker and keeps recoverable Elsewhere credentials in its private SQLite database. Protect its data directory, backups, and Docker socket as host-administrator resources. Back up the complete Innkeeper data directory together with session volumes. Login credentials are sent only in Secure cookies; the UI stores no login bearer secret in browser storage. No cross-origin API access is enabled.

Session state lives in `state.sqlite3` inside the data directory. SQLite stores sessions, launch
settings, package lists, ordered Docker options, timings, and the installation ID in relational tables.
Each session update is transactional. The database uses WAL with full synchronization; the data directory is private and
the database is readable only by its owner.

The initial schema is embedded in the binary and initialized by `rusqlite_migration` before serving requests. Before v1.0.0, schema changes use fresh state without migrations or compatibility layers. The schema runner owns SQLite's `user_version` field.

Stop Innkeeper before copying its data directory for a backup or restore. Keep the database and any
`state.sqlite3-wal` and `state.sqlite3-shm` files together, along with session volumes. Restoring the
installation ID together with its sessions preserves Docker ownership checks.

## Session lifecycle

Creation validates package names, records the session, downloads a release package if it is not cached, and prepares a stock base image. It creates a labeled volume and container, then copies the setup scripts and package into the stopped container through the Docker API. This also works when Innkeeper runs inside Docker; the source files are read from Innkeeper's filesystem.

Inside the session container, setup installs runtime services, prepares the user and runtime directories, and installs Elsewhere with `apt` or `pacman`. The package manager resolves the package's declared dependencies, including Debian recommendations. A separate script installs requested extra packages before Elsewhere starts. Innkeeper requires Elsewhere 0.11.0 and creates a private non-expiring internal credential with `elsewhere token create --admin` in the server's execution environment. It stores credentials privately and never includes them in startup logs, machine listings, or previews. Initialization includes authenticated token inventory. Browser tokens are created only through the CSRF-protected connect POST and are non-expiring. Previews use Innkeeper's internal credential. Setup and installation markers allow stopped sessions to restart without reinstalling packages.

All distributions include xterm for sessions with no extra packages. Sessions use VA-API on Intel/AMD or NVENC on NVIDIA when GPU access is enabled and software video encoding is off. Otherwise they use CPU encoders. Release packages are assumed compatible with the selected distribution; Innkeeper does not perform a separate binary or shared-library compatibility check.

Sessions default to `GSK_RENDERER=ngl` to work around GTK 4 Vulkan rendering artifacts
and `QT_QPA_PLATFORM='wayland;xcb'` so Qt tries Wayland, then X11 when its Wayland
plugin is unavailable. These settings apply to Elsewhere and applications it launches,
including applications activated through the session bus. Values already set in the
session environment, including empty values, are preserved. Innkeeper's own environment
is not forwarded to session containers.

An empty package list is valid. Package names cannot contain shell syntax, whitespace, paths, version expressions, or leading-dash options. Failed installations stop startup and expose their stage and output in the session's logs. Setup stages time out after 30 minutes; launch readiness times out after two minutes; release download and base-image preparation time out after 30 minutes. A failed session remains available for Stop and Destroy.

**Stop** terminates the container while retaining the complete session home directory. **Start** relaunches a stopped session with the same tokens and data. **Destroy** removes the owned container, home volume, and Innkeeper preparation log. It permanently deletes that session's data. Shared base images and downloaded packages remain available for reuse. Remove obsolete image tags individually with `docker image rm <exact-tag>` after checking that no retained container uses them; Innkeeper never prunes shared Docker resources. Cancelling a session during download or base-image preparation terminates that preparation process group. A cancelled preparation has no desktop to restart; destroy its record and create a new session.

Innkeeper restarts preserve sessions. Innkeeper inspects its recorded containers and reconciles exits and readiness; an interrupted preparation becomes a visible failure that can be destroyed and recreated. Container and volume deletion require a matching persistent installation label, and cleanup never uses global pruning or name-prefix deletion.

Previews use the shared screenshot API with a width in device pixels, preserving aspect ratio. Visible sessions refresh every five seconds, with at most two requests in flight. Hidden tabs and offscreen previews pause. The backend also limits captures to two concurrent requests and one request per session every two seconds. Unavailable previews leave the session controls usable.

The log view on the session page starts folded away and polls without overlapping requests while it is open and on screen. It shows the last 128 KiB of download and image-pull output and the last 1,000 Docker log lines. Docker logs rotate at 10 MiB, with three files retained. Token-bearing URL fragments and the rest of their line are redacted before output reaches the browser and before Docker persists desktop output. Stored credentials are also redacted verbatim. Token-command diagnostics are never returned to the browser.

## Supported session images

Release packages currently support x86_64 Docker hosts. Base images are reused locally; refresh them for future sessions with `docker pull archlinux:base`, `docker pull debian:13-slim`, and `docker pull ubuntu:26.04`.

- Arch Linux `archlinux:base`, rolling repositories.
- Debian 13 `debian:13-slim`, Debian 13 repositories with `main`, `contrib`, `non-free`, and `non-free-firmware` enabled.
- Ubuntu 26.04 LTS `ubuntu:26.04`, Ubuntu 26.04 repositories.

Ubuntu sessions use `elsewhere_<version>-1_ubuntu-26.04_amd64.deb`. They require an Elsewhere release that publishes that artifact. Debian packages and packages for other Ubuntu releases are not interchangeable because their FFmpeg library ABIs differ. A missing Ubuntu artifact fails the session download; Innkeeper never substitutes a Debian package. Ubuntu setup provides PipeWire and PulseAudio services, VA-API and Vulkan drivers, XWayland, and xterm. Elsewhere audio requires PipeWire 1.4.2 or later and WirePlumber 0.5.6 or later, provided by Ubuntu 26.04. The Elsewhere package declares the media-library dependencies.

The Elsewhere release version is pinned in `package.metadata.elsewhere.version` in `Cargo.toml` and embedded in the application at build time. Innkeeper generates GitHub download URLs and package filenames from that single version using the release package naming convention. Packages are cached under `packages/<version>/x86_64/<distribution>/<asset>` in Innkeeper's data directory. Downloads use HTTPS and a temporary file renamed only after a successful transfer. Concurrent session creation shares the preparation lock and reuses completed downloads. Interrupted transfers are retried on the next request.

Update that metadata field in `Cargo.toml` and rebuild Innkeeper to change the pinned release. The new package downloads during creation or an explicit upgrade, when first needed. Existing sessions retain their installed Elsewhere version. Cached packages survive Innkeeper upgrades and session destruction. Individually remove obsolete version directories from the cache when they are no longer needed; Innkeeper will download a missing package again. Restart Innkeeper after updating its installed assets.

The release package supplies Elsewhere and its accompanying notices. Innkeeper's own code uses the accompanying MIT license.

## Arch Linux and Debian packages

Release packages are attached to `vX.Y.Z` GitHub releases. Install the downloaded package
with `pacman -U ./elsewhere-innkeeper-*.pkg.tar.zst` on Arch or
`apt install ./elsewhere-innkeeper_*.deb` on Debian and Ubuntu. The package creates a
dedicated service account, installs the session assets, and provides a systemd service.
Docker must be running. Enable Innkeeper:

```sh
sudo systemctl enable --now docker elsewhere-innkeeper
```

Edit `/etc/elsewhere-innkeeper/environment` to configure native installations and restart the service. Restart `elsewhere-innkeeper` after every native package upgrade to use the updated binary and its pinned Elsewhere release. The account receives access to Docker through its supplementary `docker` group. Arch packaging builds the checkout through `packaging/arch/PKGBUILD`. Debian packaging uses `cargo-deb` with metadata in `Cargo.toml` and the service setup in `packaging/debian/`. Native source builds require Rust with edition 2024 support and Node.js 24.

## Development

The web frontend uses React and strict TypeScript. `npm run build` in `web/` runs the
type checker before bundling; `npm run typecheck` runs the checker alone. The Docker
web stage runs the same build.

```sh
docker build --target check .
```

`elsewhere-innkeeper --version` and the footer display Innkeeper's own build version. Source and Docker builds report `0.0.0-dev` unless `INNKEEPER_VERSION` is set. Docker treats an empty build argument as unset. Cargo metadata stays at `0.0.0`, including release builds. Override the displayed version with `docker build --build-arg INNKEEPER_VERSION=1.2.3 .`, `INNKEEPER_VERSION=1.2.3 docker compose up -d --build`. Elsewhere's pinned release is independent of Innkeeper's version.

## Elsewhere upgrades

Each session's page displays its installed package version, read from the container's package
metadata even while stopped. Innkeeper refreshes this information periodically and after
installation or launch. Failed inspection reports the version as unavailable.

Managers can install the preferred package on any running or stopped session. The button
says **Upgrade** when the preferred version is newer, **Downgrade** when it is older, and
**Reinstall** when the versions match or cannot be compared. Release versions, numbered Git
builds, and numeric package revisions are compared numerically.
Clicking the button opens a confirmation modal before closing running applications and leaving
the session stopped. Cancelling leaves the session untouched.

**Start** and **Relaunch** use the installed package without downloading, upgrading,
downgrading, or prompting about an available upgrade. The install action downloads the preferred
package if needed, stops a running desktop, installs the package even if its version is unchanged,
and exits. It leaves the session stopped so the user can choose **Start**. Installation does
not run the startup command or apply pending desktop settings. It retains the container and
session home volume.

Installation failures remain visible in the session's logs and error. Stop a failed session
before retrying installation. Start selects a normal launch and never retries an interrupted
installation automatically. The same install action repairs an absent or incomplete Elsewhere
installation, regardless of version. Unreadable metadata and a pending Debian package-manager
journal require manual recovery; Innkeeper reports this and blocks maintenance until it can
read settled metadata.
Installation success requires a successful maintenance exit and verification of the installed version. After 30 minutes of installation, Innkeeper
shows a warning and continues monitoring completion. Restarting Innkeeper during an upgrade
download reconnects to the existing desktop; the user can request installation again.

## Testing a local Elsewhere checkout

Run `make local` as a non-root user with Python 3.9+, Git, Make, and Docker with Buildx available.
The adjacent `../elsewhere` checkout must already exist and have the history and tags required
by its `make version` target. Innkeeper never clones or fetches that checkout.

Docker Buildx Bake builds Innkeeper and all three Elsewhere packages in parallel. Each package
build copies the adjacent checkout into its image and runs the native packaging target
with FFmpeg development libraries. The Debian builder targets Debian 13; the Ubuntu builder targets Ubuntu 26.04. The checkout's
`.dockerignore` controls which files enter the build. Docker reuses unchanged build layers;
source edits rebuild the packages inside their images. Finished packages are exported
into `.elsewhere-local/`.

Package metadata and filenames use Elsewhere's normalized Git version, including `.dirty`
for uncommitted changes. The version is derived before building and supplied to packaging
inside each image. Each build validates package metadata before exporting. The local
selection changes only after all three packages succeed. A failed build preserves the previous
selection.

After successful packaging, the command recreates the Innkeeper Compose service with the selected
manifest and packages mounted read-only. Its footer identifies local mode. The override is
read at startup. Run `make local` after source changes to build and activate them. Existing
session containers are retained. New sessions install the selected local package. Start and
Relaunch remain launch-only. Use **Reinstall** to test changed packages with the same version
or install a dirty build whose version cannot be compared. The install action always uses the
selected local package. After returning to normal mode, it installs the pinned release,
including when that requires a downgrade.
Missing local packages or an unreadable manifest cause an error rather than a GitHub download. Docker may still pull base
images and package managers may download dependencies.

Generated local configuration and exported packages live under `.elsewhere-local/`, which
is excluded by both `.gitignore` and `.dockerignore`. The override is never written to
`Cargo.toml` or embedded in an Innkeeper binary. Ordinary builds and Compose runs use the
release pin. Do not force-add the local directory.

`make elsewhere-local-reset` clears the local selection. Then run `docker compose up -d --build`
to return to the release pin. Reset does not stop or downgrade existing sessions, and it retains
staged packages for running local instances. Remove `.elsewhere-local/` manually only
when no local instance needs those files.

## Hardware encoding

New sessions default to GPU access when Innkeeper can discover a DRM render device through
`/dev/dri` and sysfs. The GPU selector shows the driver, device identity and current render node.
The lowest numbered render node is selected by default. On hybrid systems, choose the NVIDIA GPU
explicitly if that is the GPU you want Elsewhere to use. Selection controls Elsewhere's rendering
and encoding; GPU-enabled applications can still access the other exposed GPUs.

The Compose file mounts `/dev/dri` for discovery. With `docker run`, mount
`/dev/dri:/dev/dri:ro` and retain access to the host GPU metadata under `/sys`.
The Docker daemon must run on the same host. Native installations discover these devices directly.
Innkeeper resolves the selected GPU before startup. If it is unavailable, the first compatible
GPU is selected and saved. Changed GPU mappings recreate the container from a snapshot. NVIDIA
sessions require NVIDIA GPUs; other sessions support non-NVIDIA GPUs and software rendering.
If GPU access is enabled and no compatible GPU is available, startup fails before replacing anything.

Intel and AMD sessions use the image's VA-API and Vulkan drivers. NVIDIA sessions use the host's
NVIDIA driver through NVIDIA Container Toolkit. Install the host driver with DRM modesetting,
GBM/EGL support and the EGL GBM external platform library, then configure Docker:

```sh
sudo nvidia-ctk runtime configure --runtime=docker
sudo systemctl restart docker
```

For headless NVIDIA hosts, `/dev/nvidia-modeset` must exist before starting a session. Initialize it
with `sudo nvidia-modprobe -m` if needed. Steam also needs matching 32-bit NVIDIA driver libraries
on the host, such as Arch's `lib32-nvidia-utils` from multilib.
Use Arch sessions for Steam with 32-bit NVIDIA GLX; this is the verified multilib configuration.
Debian and Ubuntu sessions do not enable i386 multiarch.

Innkeeper starts NVIDIA sessions with `--runtime=nvidia`, `NVIDIA_VISIBLE_DEVICES=all` and
`NVIDIA_DRIVER_CAPABILITIES=compute,video,graphics,utility,display,compat32`.
Use the Toolkit's `legacy` runtime mode for this driver-capability configuration. This is a host-wide
setting for containers using the NVIDIA runtime; account for other workloads that depend on its automatic
or CDI mode before changing it:

```sh
sudo nvidia-ctk config --set nvidia-container-runtime.mode=legacy --in-place
```

NVIDIA sessions use a cached base image prepared with the distribution setup packages before driver
injection. Docker Buildx is required. This lets package managers install their graphics files before
the runtime supplies host-specific replacements. The image cache key includes its setup instructions;
remove the corresponding `innkeeper-session-*` image to rebuild it with current distribution packages.
Old tagged images remain until explicitly removed; remove unused tags as part of Docker disk maintenance.
Installing or upgrading Elsewhere in these prepared Arch sessions also refreshes the Arch system packages
to avoid dependency downloads from stale rolling-release package databases.

The runtime supplies the driver libraries; Innkeeper does not install a separate NVIDIA driver
inside the session. Session startup locates the injected GBM backend and sets `GBM_BACKENDS_PATH`. If the runtime
relocates the allocator library but leaves a broken relative backend link, Innkeeper creates its own
link under `/opt/innkeeper/gbm` to the injected allocator. It does not repair or replace injected files.
An `/usr/local/bin/Xwayland` wrapper reads the same root-owned configuration and executes
`/usr/bin/Xwayland`, so X11 rendering works even though Smithay clears most environment variables.
The wrapper leaves runtime-injected driver files untouched.

Check the session logs for an NVIDIA `GL Renderer:` and verified NVENC encoders. A working
`nvidia-smi` or successful encoding alone does not prove hardware rendering. The GPU, host driver
and distribution's FFmpeg build determine usable codecs; NVIDIA hardware need not support AV1
encoding. Missing libraries or unusable encoders produce startup errors. Enable software video
encoding explicitly to keep GPU rendering with CPU encoders at 30 Hz, or disable GPU access for
CPU rendering and encoding. GPU access grants the desktop user the required device groups.

Creation installs Innkeeper's pinned Elsewhere package. Start and Relaunch keep the installed
package and refresh the container entrypoint, desktop startup script, and launch settings.
GPU, package-list and Docker option changes apply through container replacement on Start or Relaunch.
The session retains its installed Elsewhere version. NVIDIA compatibility and distribution are fixed at creation.

### Verified GPU combinations

Elsewhere 0.10.1 was checked on an Arch host with Intel integrated graphics and an NVIDIA Ampere GPU,
driver 610.57.04, Container Toolkit 1.20.0 in legacy mode, and Docker 29.7.2. The NVIDIA GPU had the
second render node. All three session distributions passed NVIDIA compositor rendering, NVENC,
Xwayland direct GLX, native Wayland EGL, and browser video/audio through Innkeeper. Intel rendering
with VA-API and browser video/audio also passed on all three. Physical AMD hardware was not available
for this check.

| Session distribution | NVIDIA GBM directory used | FFmpeg libraries |
| --- | --- | --- |
| Arch | `/usr/lib/gbm` | 9.0.1 |
| Debian 13 | `/opt/innkeeper/gbm` private allocator link | 7.1.5 |
| Ubuntu 26.04 | `/opt/innkeeper/gbm` private allocator link | 8.0.1 |

The private-link cases exercised broken relative links supplied by the runtime. A separate Arch
session fixture exposed the real driver at `/usr/lib/x86_64-linux-gnu/gbm` and passed NVIDIA
GLX, Wayland EGL, and 32-bit GLX through the direct-directory branch. This simulates the Debian/Ubuntu
host library layout; it is not verification on a second host OS.

Arch's 32-bit GLX check passed against injected NVIDIA libraries, including after reinstalling both
32-bit and 64-bit Mesa/GL dispatch packages. Start, Relaunch, and real package upgrades from Elsewhere
0.10.0 to 0.10.1 retained the container and GPU configuration. Missing selected GPUs and changed
device mappings failed without substitution or container recreation.
GPU-disabled sessions passed device-isolation, lifecycle and browser video/audio checks on all three
distributions while using the NVIDIA runtime, with no real GPU devices exposed to the manager.
An additional Arch run passed with GPU inventory visible to the manager. Missing runtime,
modeset/control devices, unusable NVENC libraries and inaccessible
control-device permissions produced useful errors without changing the saved GPU or encoding preference.

## Session profiles

The [profiles directory](profiles/) contains ready-to-import JSON session configurations.
They specify packages, startup commands, display settings, and Docker options.
In **New session**, expand **Import profile**, paste a profile's JSON, and choose **Apply profile**.
Review the settings and choose **Create session**.

Profiles support `name`, `distribution` (`arch`, `debian`, or `ubuntu`), `packages` (an array of
package names), `startup_command`, `screen_size`, `kiosk`, `docker_args`, `gpu_access`, `gpu_id`, and `software_encoding`. Omitted fields use the
form defaults. Unknown fields are rejected. `screen_size` is `null` for dynamic sizing, or an object with `width`
and `height`, both even integers from 2 to 8192. Kiosk mode defaults to `false`.
The startup command runs through `sh -c` as the desktop user on each session start,
with the desktop's display and audio environment. An empty command starts no application.
Settings are saved with the session and retained when it is stopped and started.

Expand **Advanced Docker options** in New session to configure the Elsewhere session
container. Enter one complete `--flag=value` argument per line. Supported flags are
`--security-opt`, `--cap-add`, and `--cap-drop`; each can appear more than once.
Profiles and `POST /api/sessions` accept these options as a `docker_args` array of
complete `--flag=value` strings. Omitted `docker_args` defaults to an empty array.
Arguments allow up to 64 entries and 4096 bytes total, with nonempty values and no NUL
characters or line breaks. Innkeeper passes each argument directly to Docker without
shell expansion. Docker validates option values; a rejected value appears in the
session's startup error. Seccomp profile paths refer to files where Innkeeper's Docker
client runs. For containerized Innkeeper, mount custom seccomp profiles into that container.

Administrators can edit Docker options. Changes recreate the container on Start or Relaunch,
using a snapshot of its filesystem. Other managers retain the existing options when saving settings.

**GPU access** lets the desktop and applications use host GPUs. It defaults to on when a render device is discovered. Non-NVIDIA sessions can switch GPU access on or off; NVIDIA sessions require an NVIDIA GPU. Choose the GPU for Elsewhere in the GPU selector. Disabling access creates a session without GPU devices.

**Software video encoding** uses CPU encoders for the viewer stream while retaining GPU access for applications when enabled. The desktop runs at 30 Hz with software encoding. This launch setting applies on Start or Relaunch and does not recreate the container.

| GPU access | Software video encoding | Rendering and encoding |
| --- | --- | --- |
| On | Off | GPU rendering and VA-API or NVENC encoding |
| On | On | GPU rendering and CPU encoding |
| Off | On automatically | Software rendering and CPU encoding |

Profiles and `POST /api/sessions` accept `gpu_access` and `software_encoding` booleans and a nullable `gpu_id` from the `gpus` array returned by `GET /api/sessions`. Omit `gpu_id` to select the first discovered device. Explicit unknown IDs are rejected. With GPU access off, `gpu_id` must be null or omitted. Session responses include `gpu_id` and the saved `gpu` device description. Omitted GPU access follows host availability; omitted software encoding is off when GPU access is on. Without GPU access, software encoding is always saved as on. Edit Settings and `PUT /api/sessions/{id}/settings` can change GPU access and selection within the session's NVIDIA or non-NVIDIA category. An unchanged saved GPU may be unavailable when editing; startup resolves it again.

Use **Edit Settings** on a running or stopped session to change its name, screen size,
kiosk mode, software encoding, startup command, extra packages, GPU access and selection, or Docker options.
**Save Changes** updates the name immediately and saves other settings for the next launch.
Distribution stays fixed. Package additions install if absent; removing an entry leaves installed software intact.

**Settings pending** means saved settings differ from the container configuration or the last
successful desktop launch. Reverting edits clears the indicator. **Relaunch** disconnects the desktop
and closes applications; **Start** applies changes to a stopped session. Both retain installed software,
system files, the home directory, connection tokens and port. GPU, package-list and Docker option
changes stop and snapshot the container, remove it, and create a replacement from the snapshot.
Screen size, kiosk, encoding and startup-command changes reuse the container. Maintenance that
starts a container also applies pending container settings. Replacement starts a new Docker log;
the previous container's log is discarded.

Installing additional packages uses the distribution package manager; Arch synchronizes and upgrades
system packages to avoid a partial upgrade. Existing packages are never uninstalled by editing the list.

Snapshots retain tags beginning with `innkeeper-snapshot-`, with the session ID in the repository
name and a unique snapshot ID in the tag. Labels `io.innkeeper.snapshot=true`, `io.innkeeper.session`,
`io.innkeeper.snapshot-source`, `io.innkeeper.snapshot-id`, and `io.innkeeper.installation` identify their purpose and owner.
List them with `docker image ls --filter label=io.innkeeper.snapshot=true` and inspect labels with
`docker image inspect <image>`. Innkeeper retains snapshots after replacement and session destruction;
remove individual images after the session is destroyed and you no longer need its system files.
An interrupted replacement may depend on a snapshot that Docker considers unused. Do not remove
that image or run `docker image prune -a` or `docker system prune -a` while recovery is pending.
Treat a failed replacement as pending until the session successfully starts again. Default dangling-image pruning keeps tagged snapshots.
Reverting to the configured settings abandons an unfinished replacement if the original container
still exists. Any snapshot from that attempt remains tagged.
Snapshots preserve system files outside mounted volumes; the home volume is reattached rather than
copied. Later snapshots share layers, so removing old tags does not compact a session's history.

An interrupted replacement keeps its snapshot reference. Stop and Start retries the operation,
including after Innkeeper restarts. A failed commit retains the original container. Stop a failed session to edit settings before retrying from its saved snapshot. Snapshot operations have a 30-minute timeout.

The authenticated API accepts `PUT /api/sessions/{id}/settings` with all fields:
`name`, `screen_size`, `kiosk`, `software_encoding`, `startup_command`, `packages`, `docker_args`,
`gpu_access`, and `gpu_id`. Use `null` for dynamic screen sizing,
`false` to disable kiosk mode, and an empty string to clear the startup command.
Unknown or missing fields are rejected. `POST /api/sessions/{id}/relaunch` relaunches a
running session using saved settings. Session responses include `settings_pending`.

## Release versions

The release workflow runs on `vX.Y.Z` tags, checks that the tag identifies the checked-out
commit, and passes `X.Y.Z` as `INNKEEPER_VERSION` to both package builds. Arch and Debian
packages use `X.Y.Z-1`, and their binaries report `X.Y.Z`. Cargo metadata stays at `0.0.0`.
The workflow builds in Debian and Arch job containers with a Rust cache. Node.js and Rust build
images use digests to pin the Debian versions noted in their comments; tool updates require refreshing
those digests. It publishes
both packages and a Linux x86_64 tarball after verifying the installed Debian package and
running its authenticated API check on Debian 13 and Ubuntu 26.04.

After the package jobs succeed, the workflow builds the Dockerfile's `final` image for `linux/amd64`
with the same `INNKEEPER_VERSION` and pushes it to Docker Hub as `X.Y.Z` and `latest`. It creates the
GitHub release after the image push succeeds. Docker build records are not attached to the release.

Create a Docker Hub repository named `elsewhere-innkeeper` and configure these GitHub Actions settings:

| Setting | Type | Value |
| --- | --- | --- |
| `DOCKERHUB_IMAGE` | Repository variable | `<namespace>/elsewhere-innkeeper` |
| `DOCKERHUB_USERNAME` | Repository variable | Docker Hub login with access to that repository |
| `DOCKERHUB_TOKEN` | Repository secret | Docker Hub access token with read and write access |

Every tagged release updates `latest` without comparing versions. Branch and pull-request builds do
not publish images. If publishing fails, rerun the failed jobs. An image can already be available on
Docker Hub when GitHub release creation fails; rerun the failed release job to finish publication.

Local packaging uses the same `cargo-deb` and `makepkg` commands as the release workflow.
On Debian, install `cargo-deb` and the source build dependencies, then run:

```sh
make web
INNKEEPER_VERSION=0.0.0 cargo deb -p elsewhere-innkeeper --locked --deb-version 0.0.0-1
```

The Debian package appears in `target/debian/`. For a release build, use the tag's `X.Y.Z`
for both the environment variable and `--deb-version X.Y.Z-1`.
On Arch, install the PKGBUILD's dependencies and build the checkout:

```sh
cd packaging/arch
makepkg -f --noconfirm
```

The Arch package appears in `packaging/arch/` and uses its `pkgver`, which defaults to
`0.0.0`. Direct source builds report `0.0.0-dev` unless the version variable is set.
Rust tests and formatting checks run in the separate Docker check workflow on branch pushes
and pull requests. Release checks compare each binary's version with the tag. The Debian
installation check verifies the service files, conffile, disabled initial service, and an
authenticated response from a running Innkeeper instance using temporary state.

The tarball contains the binary, session scripts, README, and license. Extract it and run
from its directory, pointing Innkeeper at the included session assets and a writable data directory:

```sh
INNKEEPER_ASSETS_DIR="$PWD" INNKEEPER_DATA_DIR="$PWD/data" ./elsewhere-innkeeper
```

Docker must be installed and accessible to the account running Innkeeper.

Elsewhere advertises the assigned UDP port with hostname fallback. `INNKEEPER_RTC_ADDR` is an optional address override.

## Proxy verification

Run the checks in Docker:

```sh
docker build --target check -t innkeeper-proxy-check .
docker build --target proxy-rig -t innkeeper-proxy-rig .
docker run --rm -v /var/run/docker.sock:/var/run/docker.sock \
  --entrypoint python3 \
  innkeeper-proxy-rig /check-proxy.py
```

The proxy fixture creates two disposable containers and checks TLS, UUID routing, ownership, authorization, a large upload, an unbuffered stream longer than eight seconds, WebSocket binary/ping/close frames, and restart routing. It selects two ports in `19500`–`19999` that are not published by running Docker containers. Set `PROXY_TEST_HTTP=1` to check the plaintext listener used behind an HTTPS gateway. Set `PROXY_TEST_TIMEOUTS=1` to check a stalled backend and an active upload longer than the response-header idle timeout. Repeat on a custom bridge by adding `--network NETWORK -e PROXY_TEST_NETWORK=NETWORK`, or check native mode with `--network host -e INNKEEPER_IN_DOCKER=0`.

`scripts/check-session-refresh.py` checks session upgrades, settings, and Docker options
with disposable Arch, Debian and Ubuntu containers. Run it in `proxy-rig` with the Docker socket
and the checkout mounted at `/src`, using `python3 /src/scripts/check-session-refresh.py`.
The Docker options browser check uses the built frontend and a fixture API:

```sh
docker build --target proxy-browser -t innkeeper-proxy-browser .
docker run --rm --entrypoint node \
  -v "$PWD/scripts/check-docker-options-browser.mjs:/src/scripts/check-docker-options-browser.mjs:ro" \
  innkeeper-proxy-browser /src/scripts/check-docker-options-browser.mjs
```

Run `scripts/check-session-install-browser.mjs` in the same browser image with the script
mounted at `/src/scripts/check-session-install-browser.mjs` to check install labels, requests,
session states, and manager access. `scripts/check-sessions-browser.mjs` checks the workspace in
both layouts: the stage a working session announces, quick actions taking precedence over the card
that links to the session, navigation and browser history, the name and state filters, settings
that stay closed while a session cannot save them, a creation that answers after its form is
abandoned, and the reduced session page a non-manager receives.
`scripts/check-accounts-browser.mjs` checks the account pages: the directory, per-account identity
across a history jump between two accounts, creation and update payloads, deletion, password
changes and resets, sign-in, setup confirmation, and Administrator guards.
Mount each at the matching path under `/src/scripts/`.

Each browser check serves the response headers Innkeeper serves, including its Content Security
Policy, and fails on any resource the browser refuses. Keep those headers in step with `asset` in
`src/main.rs`.

Run the session refresh check with `--local` to verify same-version and incomparable dirty builds
using the selected local packages. Use `--distribution arch`, `--distribution debian`, or `--distribution ubuntu` to run the
session refresh check for one distribution.

`scripts/check-proxy-desktops.py` checks fresh real Arch, Debian and Ubuntu packages through the production creation and launch flow. Mount the script at `/check.py`; `proxy-rig` includes its SQLite fixture helper. For iteration without rebuilding the image, also mount `scripts/sqlite_fixture.py` at `/sqlite_fixture.py`. Run it in `proxy-rig` with the Docker socket, the session scripts, a writable directory at `/work`, and a local package manifest and artifacts at `/local`. Mount `/dev/dri` to exercise the host GPU. Publish `127.0.0.1:29301:29301` for a local browser rig. Leave `INNKEEPER_RTC_ADDR` unset to check hostname fallback, or set it to check an explicit override. It reserves ports used by unrelated Docker containers and removes only its own sessions.

Use a disposable work directory for this check. Each invocation starts with a fresh database and retains downloaded packages so the directory can be reused.

Set `PROXY_UPGRADE_FROM` to an older Elsewhere release with matching artifacts to check an actual package upgrade to the selected version. The rig installs the older package, requests the upgrade through Innkeeper, verifies the installed version, and starts the same container.

For browser checks, set `PROXY_WAIT_BROWSER=1` on that desktop rig, build the `proxy-browser` target, and run `node /src/scripts/check-proxy-browser.mjs` with host networking and the same `/work` directory after `/work/browser.json` appears. Set `PROXY_BROWSER_ORIGIN=https://localhost:29301` to exercise hostname resolution. This covers simultaneous desktops, Open and token isolation, decoded video and a non-silent audio test tone, file transfers, MCP, terminals, viewer access, direct WebRTC and WebSocket fallback. The browser writes `/work/browser-done` so the desktop rig can clean up.

To check GPU access and encoder selection in the real-desktop rig, set `PROXY_CHECK_GPU=1` and `PROXY_GPU_ACCESS=1` on `check-proxy-desktops.py` and expose `/dev/dri` to the rig. Repeat with `PROXY_GPU_ACCESS=0`, then keep that setting and run without exposing `/dev/dri`. The check verifies device access by the desktop user, Vulkan client enumeration, encoder logs, and Start/Relaunch without container replacement. `proxy-rig` includes `/check-glx32.c`; set `PROXY_CHECK_GLX32=1` to check 32-bit NVIDIA GLX and reinstall the Arch Mesa/GL dispatch packages after driver injection. Set `PROXY_GPU_DRIVER=nvidia` to select NVIDIA or `PROXY_GPU_DRIVER=i915` for Intel. Use `PROXY_WAIT_BROWSER=1` for the accompanying audio/video browser check. It uses hardware encoding by default; repeat with `PROXY_BROWSER_SOFTWARE=1` for CPU encoding with GPU rendering.

Set `PROXY_GBM_LAYOUT=debian` with `PROXY_DISTROS=arch` to exercise the Debian GBM directory using
the real injected driver. This fixture grants its session mount permissions and overmounts the normal
GBM directories with temporary filesystems; it does not alter the injected files underneath. It tests
directory lookup, not a second host operating system. `PROXY_CHECK_FAILURES=1` checks missing NVIDIA
device and encoder injection. `PROXY_DEFAULT_NVIDIA_RUNTIME=1` forces the rig's Docker creates through
the NVIDIA runtime to check GPU-disabled isolation without changing the daemon's default runtime.
With `/dev/null` exposed as `/dev/dri/renderD999`, `PROXY_CHECK_DISCOVERY_ERROR=1` checks that missing
GPU sysfs metadata is reported. These options operate only on the rig's sessions.

The port-discovery check creates and removes two disposable containers without starting them. It checks disappearance races, retained reservations and Docker errors. Run it with:

```sh
docker run --rm -v /var/run/docker.sock:/var/run/docker.sock \
  -v "$PWD/scripts:/scripts:ro" --entrypoint python3 \
  innkeeper-proxy-rig /scripts/check-fixture-ports.py
```
