# Copyright 2019 Red Hat, Inc.
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
Handles bringing the stratis service up and down.
"""
import os
import subprocess
import time

from ._dm import get_stratis_devices, remove_stratis_setup
from ._utils import process_exists, settle

_STRATISD = os.environ['STRATISD']


class Daemon:
    """
    Represents the stratis service
    """

    def __init__(self, available_fn):
        """
        Constructor
        :param available_fn: Function which returns True when daemon IPC is up.
        """
        self._service = None
        self._available = available_fn

    def start(self):
        """
        Starts the service
        :return: None, (may assert)
        """

        if self._service is None:
            # The service uses the udev db at start, we need to ensure that it
            # is in a consistent state for us to come up and find all the
            # stratis devices and assemble the pools before we start processing
            # dbus client requests.  Otherwise we have a race condition between
            # what the client expects and what the service knows about.
            settle()

            assert process_exists("stratisd") is None
            assert get_stratis_devices() == []

            service_up = False
            self._service = subprocess.Popen([_STRATISD, '--debug'])

            limit = time.time() + 120.0
            while time.time() <= limit:

                service_up = self._available()
                if service_up:
                    break
                else:
                    time.sleep(0.5)

                    # If service has exited we will bail
                    if self._service.poll() is not None:
                        break

            # see if service process still exists...
            time.sleep(1)
            if self._service.poll() is not None:
                rc = self._service.returncode
                self._service = None
                raise AssertionError(
                    "Daemon unexpectedly exited with %s" % str(rc))

            # Ensure we actually were able to communicate with dbus
            if not service_up:
                raise AssertionError("Daemon IPC did not become available")

            assert process_exists("stratisd") is not None

    def stop_remove_dm_tables(self):
        """
        Stops the services and unloads the dm tables
        :return: None (may assert)
        """
        if self._service:
            present_at_entry = process_exists("stratisd")

            self._service.terminate()
            self._service.wait()
            self._service = None

            remove_stratis_setup()

            # Raise exception after all the cleanup is done so caller can
            # continue clean up, yet still report errors.
            if present_at_entry is None:
                raise AssertionError(
                    "Stratis daemon was not running as expected!")

            if process_exists("stratisd") is not None:
                raise AssertionError("Unable to stop daemon!")

            if get_stratis_devices():
                raise AssertionError("Stratis devices exist after clean-up")
