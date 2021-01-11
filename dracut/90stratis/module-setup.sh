#!/usr/bin/bash

# called by dracut
check() {
    require_binaries stratis-min \
	    thin_check \
	    thin_repair \
	    mkfs.xfs \
	    xfs_admin \
	    xfs_growfs \
	    plymouth \
	    /usr/sbin/plymouthd \
	    || return 1
    return 255
}

# called by dracut
depends() {
    # clevis modules can be installed using the clevis-dracut package on Fedora
    echo dm clevis clevis-pin-sss clevis-pin-tang clevis-pin-tpm2
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
	    plymouth \
	    /usr/sbin/plymouthd

    # Dracut dependencies
    inst_multiple $systemdutildir/system-generators/stratis-setup-generator \
	    $systemdutildir/system/plymouth-start.service \
	    plymouth

    inst_rules "$moddir/11-stratisd.rules"
    inst_simple "$moddir/stratisd-min.service" $systemdutildir/system/stratisd-min.service
    inst_simple "$moddir/stratis-rootfs-setup" $systemdutildir/stratis-rootfs-setup
}
