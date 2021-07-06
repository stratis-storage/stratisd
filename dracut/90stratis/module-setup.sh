#!/usr/bin/bash

# called by dracut
check() {
	require_binaries stratis-min \
		/usr/libexec/stratisd-min \
		$systemdutildir/system-generators/stratis-setup-generator \
		thin_check \
		thin_repair \
		mkfs.xfs \
		xfs_admin \
		xfs_growfs \
		xfs_db \
		udevadm \
		plymouth \
		/usr/sbin/plymouthd \
		/usr/lib/udev/stratis-str-cmp \
		/usr/lib/udev/stratis-base32-decode ||
		return 1
	return 255
}

# called by dracut
depends() {
	echo dm
	return 0
}

# called by dracut
installkernel() {
	instmods xfs dm_crypt dm-thin-pool
}

# called by dracut
install() {
	# Stratis dependencies
	inst_multiple stratis-min \
		/usr/libexec/stratisd-min \
		thin_check \
		thin_repair \
		mkfs.xfs \
		xfs_admin \
		xfs_growfs \
		xfs_db \
		udevadm \
		plymouth \
		/usr/sbin/plymouthd \
		/usr/lib/udev/stratis-str-cmp
	ln ${initdir}/usr/lib/udev/stratis-str-cmp ${initdir}/usr/lib/udev/stratis-base32-decode

	# Dracut dependencies
	inst_multiple $systemdutildir/system-generators/stratis-setup-generator \
		$systemdutildir/system/plymouth-start.service \
		plymouth

	inst_rules "$moddir/61-stratisd.rules"
	inst_simple "$moddir/stratisd-min.service" $systemdutildir/system/stratisd-min.service
	inst_simple "$moddir/stratis-rootfs-setup" $systemdutildir/stratis-rootfs-setup
}
