#!/bin/bash
# Build a binary debian package 'in place' using a debian provided docker image
DISTRO_VERSION=${DISTRO_VERSION:="buster"}
SRCDIR=$(cd $(dirname $0)/.. && pwd)
BUILDSCRIPTNAME="build.sh"
TARGETDIR="target/debian"
MOUNTPATH="/pkg"

mkdir -p "${SRCDIR}/${TARGETDIR}"

cat << EOF > "${SRCDIR}/${TARGETDIR}/${BUILDSCRIPTNAME}"
#!/bin/bash
apt-get update
apt-get install -y \$(grep "^Build-Depends: " ${MOUNTPATH}/debian/control | sed -e "s/^Build-Depends: //" -e "s/([^)]*)//" -e "s/,//g")
if [ \$? -ne 0 ]; then echo "dependencies install failed";exit 10;fi
cd ${MOUNTPATH}
PKGVERSION=\$(cargo pkgid | sed -e "s/^.*://")
cat << EOCL > debian/changelog
stratisd (\${PKGVERSION}) ${DISTRO_VERSION}; urgency=low

  * Update to version \${PKGVERSION}

 -- Gert Dewit <gertux@hobbiton.be>  \$(date "+%a, %d %b %Y %H:%M:%S %z")

EOCL
dpkg-buildpackage -us -ui -uc -b
if [ \$? -ne 0 ]; then echo "package build failed";exit 20;fi
mv ../*.deb ${TARGETDIR}

OWNER=\$(ls -lnd ${MOUNTPATH} | awk '{ print \$3 }')
if [ -z \${OWNER} ]
then
  echo "Cannot change ownerships"
else
  chown -R \${OWNER} "${MOUNTPATH}/debian" "${MOUNTPATH}/${TARGETDIR}"
fi 
EOF

chmod +x "${SRCDIR}/${TARGETDIR}/${BUILDSCRIPTNAME}"

docker run -i --rm --volume="${SRCDIR}:${MOUNTPATH}" debian:${DISTRO_VERSION} "${MOUNTPATH}/${TARGETDIR}/${BUILDSCRIPTNAME}"


