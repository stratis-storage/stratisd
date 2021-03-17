#!/usr/bin/python3
#
# Copyright 2018 Red Hat, Inc.
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
"""
Identify a device as a Stratis block device.
"""
# isort: STDLIB
import struct
import sys
from collections import namedtuple

# isort: THIRDPARTY
from _crc32 import crc

BS = 512
FIRST_COPY_OFFSET = BS
SECOND_COPY_OFFSET = BS * 9
SB_AREA_SIZE = 16 * BS
STRATIS_MAGIC = b"!Stra0tis\x86\xff\x02^Arh"

MAGIC_OFFSET = 4
MAGIC_LEN = len(STRATIS_MAGIC)


def _valid_stratis_sb(buf):
    """
    Check to see if the buffer is a valid Stratis super block
    :param buf: Byte buffer starting at Stratis block offset
    :return: None or named tuple
    """
    if buf[MAGIC_OFFSET : MAGIC_OFFSET + MAGIC_LEN] == STRATIS_MAGIC:
        # Verify CRC
        if crc(buf[MAGIC_OFFSET:BS]) == struct.unpack_from("<L", buf, 0)[0]:
            super_block = namedtuple(
                "StratisSuperblock",
                "CRC32C STRATIS_MAGIC_NUMBER BLKDEV_SIZE_SECTORS "
                "SIGBLOCK_VERSION UNUSED POOL_UUID DEV_UUID MDA_SIZE_SECTORS "
                "RESERVED_SIZE_SECTORS FLAGS INITIALIZATION_TIME_SECONDS",
            )

            return super_block._make(struct.unpack_from("<L16sQc3s32s32sQQQQ", buf))
    return None


def stratis_signature(block_device):
    """
    Checks a device to see if it has a valid Stratis signature on it.
    :param block_device:
    :return: None if not Stratis, else named tuple
    """
    try:
        with open(block_device, "r+b") as header:
            buf = header.read(SB_AREA_SIZE)
    # pylint: disable=bare-except
    except:
        return None

    return _valid_stratis_sb(buf[FIRST_COPY_OFFSET:]) or _valid_stratis_sb(
        buf[SECOND_COPY_OFFSET:]
    )


def main():
    """
    The main method
    """
    if len(sys.argv) != 2:
        print("syntax: stratis_signature.py <block device>")
        sys.exit(2)

    sig = stratis_signature(sys.argv[1])
    if not sig:
        sys.exit(1)

    print(sig)
    sys.exit(0)


if __name__ == "__main__":
    main()
