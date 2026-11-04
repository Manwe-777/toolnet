FROM debian:13
RUN apt-get update -q && apt-get install -y -q systemd systemd-resolved curl sudo && apt-get clean
RUN systemctl mask getty@.service serial-getty@.service console-getty.service systemd-vconsole-setup.service systemd-udevd.service systemd-udev-trigger.service systemd-logind.service 2>/dev/null; true
COPY boot.sh /boot.sh
ENTRYPOINT ["/bin/sh", "/boot.sh"]
