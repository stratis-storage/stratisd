#!/bin/bash

# Migrate the symlinks of Stratis filesystems from the old
# /stratis path to /dev/stratis, via synthetically generating
# udev change events.

if [ ! -d /stratis ]
then
	echo "'/stratis' directory does not exist; nothing to do."
	exit 0
fi

if [ ! -x /usr/lib/udev/stratis_uuids_to_names ]
then
	echo "stratis_uuids_to_names' udev program does not exist."
	exit 2
fi

if [ ! -x /usr/bin/stratis_dbusquery_version ]
then
	echo "'stratis_dbusquery_version' program does not exist."
	exit 2
fi

if [ ! -f /usr/lib/udev/rules.d/11-stratisd.rules ]
then
	echo "stratisd udev rule file does not exist in /usr/lib/udev/rules.d"
	exit 2
fi

/usr/bin/stratis_dbusquery_version
RC_DBUSQUERY=$?

if [ $RC_DBUSQUERY == 2 ]
then
	echo "Attempt to query stratisd version over dbus failed."
	exit 2
fi

if [ $RC_DBUSQUERY == 1 ]
then
	echo "stratisd reported version is too old."
	exit 1
fi

for i in $(find /stratis)
do
	if [ ! -d $i ]
	then
		echo "Checking file $i..."
	fi
	if [ -h $i ] && [ -b $i ]
	then
		devname=$i
		echo "Link name: $devname"
		linktgt=$(readlink $devname)
		tgtbase=$(basename $linktgt)
		echo "Link target: $linktgt"
		echo "Target base name: $tgtbase"
		futurename="/dev$devname"
		echo "Future name: $futurename"
		if [ -h $futurename ] && [ -b $futurename ]
		then
			echo "Link seems to already exist"
			# Paths from udev are set relative
			# to /dev/stratis, so use readlink -e.
			futuretgt=$(readlink -e $futurename)
			echo "Future link target: $futuretgt"
			if [ ! $futuretgt -ef $linktgt ]
			then
				echo "Targets do not match; sending change event to re-synchronize..."
				udevadm test --action=change /sys/class/block/$tgtbase 1>/dev/null 2>&1
			else
				echo "Targets match"
			fi
		else
			echo "No future link found; sending change event..."
			udevadm test --action=change /sys/class/block/$tgtbase 1>/dev/null 2>&1
		fi
	fi	
done

rm -rfv /stratis/
