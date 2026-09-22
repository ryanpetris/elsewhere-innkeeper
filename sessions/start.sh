#!/bin/sh
set -eu
. /opt/innkeeper/launch-settings.sh
. /opt/innkeeper/gpu-settings.sh
. /opt/innkeeper/gpu-env.sh
export PATH=/usr/local/bin:/usr/local/sbin:/usr/bin:/usr/sbin:/bin:/sbin
if [ "$INNKEEPER_RENDER_NODE" != none ]; then
    if [ ! -c "$INNKEEPER_RENDER_NODE" ] || [ ! -r "$INNKEEPER_RENDER_NODE" ] || [ ! -w "$INNKEEPER_RENDER_NODE" ]; then
        echo 'Selected GPU is inaccessible in this container. Check device permissions, then Stop and Start.' >&2
        exit 1
    fi
    major=$(stat -c %t "$INNKEEPER_RENDER_NODE")
    minor=$(stat -c %T "$INNKEEPER_RENDER_NODE")
    device=$(printf '%d:%d' "0x$major" "0x$minor")
    identity=$(basename "$(readlink -f "/sys/dev/char/$device/device")")
    driver=$(basename "$(readlink -f "/sys/dev/char/$device/device/driver")")
    if [ "$device" != "$INNKEEPER_GPU_DEVICE" ] || [ "$identity" != "$INNKEEPER_GPU_ID" ] || [ "$driver" != "$INNKEEPER_GPU_DRIVER" ]; then
        echo 'Selected GPU device mapping changed. Stop and Start to refresh the GPU mapping.' >&2
        exit 1
    fi
fi
if [ "$INNKEEPER_GPU_DRIVER" = nvidia ]; then
    for device in /dev/nvidiactl /dev/nvidia-modeset; do
        if [ ! -r "$device" ] || [ ! -w "$device" ]; then
            echo "NVIDIA device $device is inaccessible to the desktop user. Check host device permissions." >&2
            exit 1
        fi
    done
fi
export HOME=/home/elsewhere XDG_CONFIG_HOME=/home/elsewhere/.config XDG_RUNTIME_DIR=/tmp/runtime-elsewhere NO_COLOR=1
export GSK_RENDERER="${GSK_RENDERER-ngl}" QT_QPA_PLATFORM="${QT_QPA_PLATFORM-wayland;xcb}"
cd "$HOME"
DBUS_SESSION_BUS_ADDRESS=$(dbus-daemon --session --fork --print-address)
export DBUS_SESSION_BUS_ADDRESS
rm -f "$XDG_RUNTIME_DIR/output"
mkfifo "$XDG_RUNTIME_DIR/output"
# Redact credential fragments before Docker persists application output.
LC_ALL=C stdbuf -oL sed -E 's/(#token=).*/\1[REDACTED]/' < "$XDG_RUNTIME_DIR/output" &
filter=$!
set -- --no-tls --listen 0.0.0.0:19443 --url-prefix "$INNKEEPER_URL_PREFIX" --rtc-port "$INNKEEPER_RTC_PORT" --elements --render-node "$INNKEEPER_RENDER_NODE"
if [ -n "${INNKEEPER_RTC_ADDR:-}" ]; then set -- "$@" --rtc-addr "$INNKEEPER_RTC_ADDR"; fi
if [ -n "${INNKEEPER_SCREEN_SIZE:-}" ]; then set -- "$@" --screen-size "$INNKEEPER_SCREEN_SIZE"; fi
if [ "${INNKEEPER_SOFTWARE_ENCODING}" = 1 ]; then set -- "$@" --software-encoding; fi
if [ "${INNKEEPER_KIOSK:-0}" = 1 ]; then set -- "$@" --kiosk; fi
if [ -n "${INNKEEPER_STARTUP_COMMAND:-}" ]; then set -- "$@" --exec "$INNKEEPER_STARTUP_COMMAND"; fi
elsewhere "$@" > "$XDG_RUNTIME_DIR/output" 2>&1 &
desktop=$!
trap 'kill -TERM "$desktop" 2>/dev/null || true' TERM INT
set +e
wait "$desktop"
status=$?
if kill -0 "$desktop" 2>/dev/null; then
    wait "$desktop"
    status=$?
fi
# Drain crash output; an application inheriting stdout cannot hold shutdown forever.
(sleep 2; kill "$filter" 2>/dev/null) &
drain_timeout=$!
wait "$filter"
kill "$drain_timeout" 2>/dev/null
wait "$drain_timeout" 2>/dev/null
exit "$status"
