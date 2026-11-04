FROM fedora:43
RUN dnf install -y -q systemd systemd-resolved curl && dnf clean all
RUN systemctl mask getty@.service serial-getty@.service console-getty.service systemd-vconsole-setup.service systemd-udevd.service systemd-udev-trigger.service systemd-logind.service 2>/dev/null; true
COPY boot.sh /boot.sh
ENTRYPOINT ["/bin/sh", "/boot.sh"]
