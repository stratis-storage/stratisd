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
	    jose \
	    jq \
	    cryptsetup \
	    curl \
	    tpm2_createprimary \
	    tpm2_unseal \
	    tpm2_load \
	    clevis \
	    clevis-luks-list \
	    clevis-luks-bind \
	    clevis-luks-unlock \
	    clevis-luks-unbind \
	    clevis-encrypt-tang \
	    clevis-encrypt-tpm2 \
	    clevis-decrypt \
	    clevis-decrypt-tang \
	    clevis-decrypt-tpm2 \
	    clevis-luks-common-functions \
	    || return 1
    require_any_binary tpm2_pcrread tpm2_pcrlist || return 1
    return 255
}

# called by dracut
depends() {
    echo dm
    return 0
}

# called by dracut
installkernel() {
    instmods xfs dm_crypt dm-thin-pool tpm
    hostonly='' instmods =drivers/char/tpm
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
	    /usr/sbin/plymouthd \
	    clevis \
	    clevis-luks-list \
	    clevis-luks-bind \
	    clevis-luks-unlock \
	    clevis-luks-unbind \
	    clevis-encrypt-tang \
	    clevis-encrypt-tpm2 \
	    clevis-decrypt \
	    clevis-decrypt-tang \
	    clevis-decrypt-tpm2 \
	    clevis-luks-common-functions \
	    tpm2_createprimary \
	    tpm2_unseal \
	    tpm2_load \
	    jose \
	    jq \
	    cryptsetup \
	    curl
    inst_multiple -o tpm2_pcrread tpm2_pcrlist
    inst_libdir_file "libtss2-tcti-device.so*"

    # Dracut dependencies
    inst_multiple $systemdutildir/system-generators/stratis-setup-generator \
	    $systemdutildir/system/plymouth-start.service \
	    plymouth

    inst_rules "$moddir/11-stratisd.rules"
    inst_simple "$moddir/stratisd-min.service" $systemdutildir/system/stratisd-min.service
    inst_simple "$moddir/stratis-rootfs-setup" $systemdutildir/stratis-rootfs-setup
}
