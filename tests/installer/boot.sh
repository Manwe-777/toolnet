#!/bin/sh
# Container entrypoint: systemd needs a writable cgroup tree, which Docker only
# gives privileged containers. These tests must NOT run privileged: a
# privileged systemd grabs the host's real /dev (VTs, udev) and took down the
# desktop session once.
mount -o remount,rw /sys/fs/cgroup
exec /usr/lib/systemd/systemd
