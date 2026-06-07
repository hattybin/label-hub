#!/bin/bash -e
# pi-gen stage prerun: ensure we build on top of the previous stage's rootfs.
if [ ! -d "${ROOTFS_DIR}" ]; then
	copy_previous
fi
