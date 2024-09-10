# Copyright 2024 Red Hat, Inc.
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
Test reverting a filesystem.
"""

# isort: STDLIB
import os
import subprocess
import tempfile

# isort: THIRDPARTY
from justbytes import Range

# isort: FIRSTPARTY
from dbus_python_client_gen import DPClientInvocationError

# isort: LOCAL
from stratisd_client_dbus import Filesystem
from stratisd_client_dbus._constants import TOP_OBJECT

from ._utils import (
    Manager,
    Pool,
    ServiceContextManager,
    UdevTest,
    create_pool,
    get_object,
    random_string,
    settle,
)


class TestRevert(UdevTest):
    """
    Test reverting a filesystem.
    """

    def test_revert(self):  # pylint: disable=too-many-locals
        """
        Schedule a revert and verify that it has succeeded when the pool is
        restarted.

        First simply stop and start stratisd. In this way it is possible to
        verify that when a revert fails, the pool is setup, without the revert.
        """
        mountdir = tempfile.mkdtemp("_stratis_mnt")

        with ServiceContextManager():
            device_tokens = self._lb_mgr.create_devices(2)

            pool_name = random_string(5)

            (_, (pool_object_path, _)) = create_pool(
                pool_name, self._lb_mgr.device_files(device_tokens)
            )

            fs_name = "fs1"
            fs_size = Range(1024**3)
            ((_, fs_object_paths), return_code, message) = (
                Pool.Methods.CreateFilesystems(
                    get_object(pool_object_path),
                    {"specs": [(fs_name, (True, str(fs_size.magnitude)), (False, ""))]},
                )
            )

            if return_code != 0:
                raise RuntimeError(
                    f"Failed to create a requested filesystem: {message}"
                )

            settle()

            filepath = f"/dev/stratis/{pool_name}/{fs_name}"
            subprocess.check_call(["mount", filepath, mountdir])

            file1 = "file1.txt"
            with open(os.path.join(mountdir, file1), encoding="utf-8", mode="w") as fd:
                print(file1, file=fd, end="")

            snap_name = "snap1"
            ((_, snap_object_path), return_code, message) = (
                Pool.Methods.SnapshotFilesystem(
                    get_object(pool_object_path),
                    {"origin": fs_object_paths[0][0], "snapshot_name": snap_name},
                )
            )

            if return_code != 0:
                raise RuntimeError(f"Failed to create requested snapshot: {message}")

            file2 = "file2.txt"
            with open(os.path.join(mountdir, file2), encoding="utf-8", mode="w") as fd:
                print(file2, file=fd, end="")

            Filesystem.Properties.MergeScheduled.Set(get_object(snap_object_path), True)
            subprocess.check_call(["umount", mountdir])

            self.assertTrue(os.path.exists(f"/dev/stratis/{pool_name}/{snap_name}"))

        # Do not stop the pool, but do stop stratisd. Since the devices were
        # not torn down, the merge will fail and both filesystems will be set
        # up as they were previously.
        with ServiceContextManager():
            self.wait_for_pools(1)

            settle()

            subprocess.check_call(["mount", filepath, mountdir])

            with open(os.path.join(mountdir, file1), encoding="utf-8") as fd:
                self.assertEqual(fd.read(), file1)

            with open(os.path.join(mountdir, file2), encoding="utf-8") as fd:
                self.assertEqual(fd.read(), file2)

            subprocess.check_call(["umount", mountdir])

            # Now stop the pool, which should tear down the devices
            (_, return_code, message) = Manager.Methods.StopPool(
                get_object(TOP_OBJECT),
                {
                    "id": pool_name,
                    "id_type": "name",
                },
            )

            if return_code != 0:
                raise RuntimeError(f"Failed to stop the pool {pool_name}: {message}")

            (_, return_code, message) = Manager.Methods.StartPool(
                get_object(TOP_OBJECT),
                {
                    "id": pool_name,
                    "id_type": "name",
                    "unlock_method": (False, ""),
                    "key_fd": (False, 0),
                },
            )

            if return_code != 0:
                raise RuntimeError(f"Failed to start the pool {pool_name}: {message}")

            self.wait_for_pools(1)

            settle()

            subprocess.check_call(["mount", filepath, mountdir])

            with open(os.path.join(mountdir, file1), encoding="utf-8") as fd:
                self.assertEqual(fd.read(), file1)

            self.assertFalse(os.path.exists(os.path.join(mountdir, file2)))
            self.assertFalse(os.path.exists(f"/dev/stratis/{pool_name}/{snap_name}"))

            subprocess.check_call(["umount", mountdir])

    def test_revert_snapshot_chain(self):  # pylint: disable=too-many-locals
        """
        Make a chain of snapshots, schedule excess reverts and verify that
        those yield an error, and then revert the middle link.

        Verify that the snapshot link now points to the origin.
        """
        mountdir = tempfile.mkdtemp("_stratis_mnt")

        with ServiceContextManager():
            device_tokens = self._lb_mgr.create_devices(2)

            pool_name = random_string(5)

            (_, (pool_object_path, _)) = create_pool(
                pool_name, self._lb_mgr.device_files(device_tokens)
            )

            fs_name = "fs1"
            fs_size = Range(1024**3)
            ((_, fs_object_paths), return_code, message) = (
                Pool.Methods.CreateFilesystems(
                    get_object(pool_object_path),
                    {"specs": [(fs_name, (True, str(fs_size.magnitude)), (False, ""))]},
                )
            )

            if return_code != 0:
                raise RuntimeError(
                    f"Failed to create a requested filesystem: {message}"
                )

            settle()

            filepath = f"/dev/stratis/{pool_name}/{fs_name}"
            subprocess.check_call(["mount", filepath, mountdir])

            file1 = "file1.txt"
            with open(os.path.join(mountdir, file1), encoding="utf-8", mode="w") as fd:
                print(file1, file=fd, end="")

            snap_name_1 = "snap1"
            ((_, snap_object_path_1), return_code, message) = (
                Pool.Methods.SnapshotFilesystem(
                    get_object(pool_object_path),
                    {"origin": fs_object_paths[0][0], "snapshot_name": snap_name_1},
                )
            )

            if return_code != 0:
                raise RuntimeError(f"Failed to create requested snapshot: {message}")

            file2 = "file2.txt"
            with open(os.path.join(mountdir, file2), encoding="utf-8", mode="w") as fd:
                print(file2, file=fd, end="")

            Filesystem.Properties.MergeScheduled.Set(
                get_object(snap_object_path_1), True
            )
            subprocess.check_call(["umount", mountdir])

            self.assertTrue(os.path.exists(f"/dev/stratis/{pool_name}/{snap_name_1}"))

            snap_name_2 = "snap2"
            ((_, snap_object_path_2), return_code, message) = (
                Pool.Methods.SnapshotFilesystem(
                    get_object(pool_object_path),
                    {"origin": snap_object_path_1, "snapshot_name": snap_name_2},
                )
            )

            if return_code != 0:
                raise RuntimeError(f"Failed to create requested snapshot: {message}")

            settle()

            self.assertTrue(os.path.exists(f"/dev/stratis/{pool_name}/{snap_name_2}"))
            with self.assertRaises(DPClientInvocationError):
                Filesystem.Properties.MergeScheduled.Set(
                    get_object(snap_object_path_2), True
                )

            # Now stop the pool, which should tear down the devices
            (_, return_code, message) = Manager.Methods.StopPool(
                get_object(TOP_OBJECT),
                {
                    "id": pool_name,
                    "id_type": "name",
                },
            )

            if return_code != 0:
                raise RuntimeError(f"Failed to stop the pool {pool_name}: {message}")

            (_, return_code, message) = Manager.Methods.StartPool(
                get_object(TOP_OBJECT),
                {
                    "id": pool_name,
                    "id_type": "name",
                    "unlock_method": (False, ""),
                    "key_fd": (False, 0),
                },
            )

            if return_code != 0:
                raise RuntimeError(f"Failed to start the pool {pool_name}: {message}")

            self.wait_for_pools(1)

            settle()

            subprocess.check_call(["mount", filepath, mountdir])

            with open(os.path.join(mountdir, file1), encoding="utf-8") as fd:
                self.assertEqual(fd.read(), file1)

            self.assertFalse(os.path.exists(os.path.join(mountdir, file2)))
            self.assertFalse(os.path.exists(f"/dev/stratis/{pool_name}/{snap_name_1}"))
            self.assertTrue(os.path.exists(f"/dev/stratis/{pool_name}/{snap_name_2}"))

            subprocess.check_call(["umount", mountdir])
