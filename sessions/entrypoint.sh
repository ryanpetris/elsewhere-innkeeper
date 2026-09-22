#!/bin/sh
set -eu
: > /tmp/innkeeper-timings
stage() { echo "Stage: $1"; printf '%s\n' "$1" > /tmp/innkeeper-stage; printf '%s %s\n' "$(date +%s%3N)" "$1" >> /tmp/innkeeper-timings; }
trap 'echo "Setup failed during $(cat /tmp/innkeeper-stage)" >&2' EXIT
mode=launch
attempt=
expected=
if [ -f /opt/innkeeper/operation ]; then
    read -r mode attempt expected < /opt/innkeeper/operation
fi
# An ordinary container start cannot repeat a maintenance request.
printf 'launch\n' > /opt/innkeeper/operation
stage setup
if [ ! -f /opt/innkeeper/setup-complete ]; then
    if [ "${INNKEEPER_BASE_READY:-0}" != 1 ]; then sh /opt/innkeeper/setup.sh; fi
    touch /opt/innkeeper/setup-complete
fi
if [ "$mode" = upgrade ]; then
    stage upgrade
    sh /opt/innkeeper/install.sh "$expected"
    printf '%s\n' "$attempt" > /opt/innkeeper/upgrade-complete
    trap - EXIT
    exit 0
fi
mkdir -p /home/elsewhere/.config/elsewhere /tmp/runtime-elsewhere /tmp/.X11-unix
chmod 1777 /tmp/.X11-unix
chown -R elsewhere:elsewhere /home/elsewhere /tmp/runtime-elsewhere
chmod 700 /tmp/runtime-elsewhere /home/elsewhere/.config/elsewhere
if [ "$mode" = create ]; then
    stage elsewhere
    sh /opt/innkeeper/install.sh "$expected"
fi
stage packages
if [ ! -f /opt/innkeeper/packages-installed ] || [ "$(cat /opt/innkeeper/packages-requested)" != "$(cat /opt/innkeeper/packages-installed)" ]; then
    sh /opt/innkeeper/packages.sh "$@"
    cp /opt/innkeeper/packages-requested /opt/innkeeper/packages-installed
fi
# Device group IDs come from the host and may differ from the image's groups.
for device in /dev/dri/card* /dev/dri/renderD* /dev/nvidia* /dev/nvidia-caps/*; do
    [ -c "$device" ] || continue
    gid=$(stat -c %g "$device")
    [ "$gid" -eq 0 ] && continue
    group=$(getent group "$gid" | cut -d: -f1)
    if [ -z "$group" ]; then
        group="innkeeper-gpu-$gid"
        groupadd -g "$gid" "$group"
    fi
    usermod -aG "$group" elsewhere
done
stage launch
sh /opt/innkeeper/gpu.sh
trap - EXIT
exec runuser -u elsewhere -- sh /opt/innkeeper/start.sh
