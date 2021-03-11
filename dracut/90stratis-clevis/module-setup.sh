#!/usr/bin/bash
# This dracut module requires the kernel command line parameter rd.neednet=1 if
# the root filesystem is hosted on a LUKS2 volume bound to a Tang server.

# called by dracut
check() {
    require_binaries jose \
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
	    mktemp \
	    $systemdutildir/system-generators/stratis-clevis-setup-generator \
	    || return 1
    require_any_binary tpm2_pcrread tpm2_pcrlist || return 1
    return 255
}

# called by dracut
depends() {
    echo stratis
    return 0
}

# called by dracut
installkernel() {
    hostonly='' instmods =drivers/char/tpm
}

# called by dracut
install() {
    # Clevis dependencies
    inst_multiple clevis \
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
	    mktemp \
	    curl
    inst_multiple -o tpm2_pcrread tpm2_pcrlist
    inst_libdir_file "libtss2-tcti-device.so*"

    # Dracut dependencies
    inst_multiple $systemdutildir/system-generators/stratis-clevis-setup-generator
    inst_simple "$moddir/stratis-clevis-rootfs-setup" $systemdutildir/stratis-clevis-rootfs-setup
}
