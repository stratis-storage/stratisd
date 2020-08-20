#!/usr/bin/bash

# called by dracut
check() {
    require_binaries stratis-min thin_check thin_repair mkfs.xfs xfs_admin xfs_growfs || return 1
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
    inst_multiple stratis-min thin_check thin_repair mkfs.xfs xfs_admin xfs_growfs

    inst_hook initqueue/settled 25 "$moddir/stratis-encrypted-rootfs.sh"
    inst_rules "$moddir/11-stratisd.rules"
}

