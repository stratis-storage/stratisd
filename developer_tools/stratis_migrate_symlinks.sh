#!/bin/bash

# Migrate the symlinks of Stratis filesystems from the old
# /stratis path to /dev/stratis, via synthetically generating
# udev change events.

if [ ! -d /stratis ]
then
	echo "'/stratis' directory does not exist; nothing to do."
	exit 0
fi

for i in $(find /stratis)
do
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
				rm -fv $devname
			else
				echo "Targets match"
				rm -fv $devname
			fi
		else
			echo "No future link found; sending change event..."
			udevadm test --action=change /sys/class/block/$tgtbase 1>/dev/null 2>&1
			rm -fv $devname
		fi
	fi	
done

rm -rfv /stratis/
