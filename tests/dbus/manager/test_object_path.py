# Copyright 2016 Red Hat, Inc.
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
Test object path methods.
"""
import time
import unittest

from stratisd_client_dbus import Manager
from stratisd_client_dbus import Pool
from stratisd_client_dbus import StratisdErrorsGen
from stratisd_client_dbus import get_object

from stratisd_client_dbus._constants import TOP_OBJECT

from .._constants import _DEVICES

from .._misc import checked_call
from .._misc import _device_list
from .._misc import Service

_MN = Manager.MethodNames
_PN = Pool.MethodNames

class GetObjectTestCase(unittest.TestCase):
    """
    Test get_object method.
    """

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        self._service = Service()
        self._service.setUp()
        time.sleep(1) # wait until the service is available

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testNonExisting(self):
        """
        A proxy object is returned from a non-existant path.
        """
        self.assertIsNotNone(get_object('/this/is/not/an/object/path'))

    def testInvalid(self):
        """
        An invalid path causes an exception to be raised.
        """
        with self.assertRaises(ValueError):
            get_object('abc')


class GetPoolTestCase(unittest.TestCase):
    """
    Test get_pool method when there is no pool.
    """

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        self._service = Service()
        self._service.setUp()
        time.sleep(1)
        self._proxy = get_object(TOP_OBJECT)

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testNonExistingPool(self):
        """
        An error code is returned if the pool does not exist.
        """
        (_, rc, _) = \
           checked_call(Manager, self._proxy, _MN.GetPoolObjectPath, "notapool")
        expected_rc = StratisdErrorsGen.get_object().POOL_NOTFOUND
        self.assertEqual(rc, expected_rc)


class GetPool1TestCase(unittest.TestCase):
    """
    Test get_pool method when there is a pool.
    """
    _POOLNAME = 'deadpool'

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        self._service = Service()
        self._service.setUp()
        time.sleep(1)
        self._proxy = get_object(TOP_OBJECT)
        Manager.callMethod(
           self._proxy,
           _MN.CreatePool,
           self._POOLNAME,
           0,
           False,
           [d.device_node for d in _device_list(_DEVICES, 1)]
        )

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testExecution(self):
        """
        Getting an existing pool should succeed.
        """
        (result, rc, _) = checked_call(
           Manager,
           self._proxy,
           _MN.GetPoolObjectPath,
           self._POOLNAME
        )
        self.assertEqual(rc, StratisdErrorsGen.get_object().OK)
        self.assertNotEqual(result, '')

    def testUnknownName(self):
        """
        Getting a non-existing pool should fail.
        """
        (_, rc, _) = \
           checked_call(Manager, self._proxy, _MN.GetPoolObjectPath, 'nopool')
        expected_rc = StratisdErrorsGen.get_object().POOL_NOTFOUND
        self.assertEqual(rc, expected_rc)


class GetVolumeTestCase(unittest.TestCase):
    """
    Test get_volume method when there is no pool.
    """

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        self._service = Service()
        self._service.setUp()
        time.sleep(1)
        self._proxy = get_object(TOP_OBJECT)

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testNonExistingPool(self):
        """
        If the pool does not exist, the filesystem is not found.

        Given our implementation, it is impossible to distinguish whether that
        is because the filesystem is not found or because the pool is not found.
        """
        (_, rc, _) = checked_call(
           Manager,
           self._proxy,
           _MN.GetFilesystemObjectPath,
           'notapool',
           'noname'
        )
        expected_rc = StratisdErrorsGen.get_object().FILESYSTEM_NOTFOUND
        self.assertEqual(rc, expected_rc)


class GetVolume1TestCase(unittest.TestCase):
    """
    Test get_volume method when there is a pool but no volume.
    """
    _POOLNAME = 'deadpool'

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        self._service = Service()
        self._service.setUp()
        time.sleep(1)
        self._proxy = get_object(TOP_OBJECT)
        Manager.callMethod(
           self._proxy,
           _MN.CreatePool,
           self._POOLNAME,
           0,
           False,
           [d.device_node for d in _device_list(_DEVICES, 1)]
        )

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testNonExistingVolume(self):
        """
        An exception is raised if the volume does not exist.
        """
        (_, rc, _) = checked_call(
           Manager,
           self._proxy,
           _MN.GetFilesystemObjectPath,
           self._POOLNAME,
           'noname'
        )
        expected_rc = StratisdErrorsGen.get_object().FILESYSTEM_NOTFOUND
        self.assertEqual(rc, expected_rc)


class GetVolume2TestCase(unittest.TestCase):
    """
    Test get_volume method when there is a pool and the volume is there.
    """
    _POOLNAME = 'deadpool'
    _VOLNAME = 'vol'

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        self._service = Service()
        self._service.setUp()
        time.sleep(1)
        self._proxy = get_object(TOP_OBJECT)
        (poolpath, _, _) = Manager.callMethod(
           self._proxy,
           _MN.CreatePool,
           self._POOLNAME,
           0,
           False,
           [d.device_node for d in _device_list(_DEVICES, 1)]
        )
        Pool.callMethod(
           get_object(poolpath),
           _PN.CreateFilesystems,
           [(self._VOLNAME, '', 0)]
        )

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testExistingVolume(self):
        """
        The volume should be discovered.
        """
        (result, rc, _) = checked_call(
           Manager,
           self._proxy,
           _MN.GetFilesystemObjectPath,
           self._POOLNAME,
           self._VOLNAME
        )
        expected_rc = StratisdErrorsGen.get_object().OK
        self.assertEqual(rc, expected_rc)
        self.assertNotEqual(result, "")

    def testNonExistingVolume(self):
        """
        The volume does not exists.
        """
        (_, rc, _) = checked_call(
           Manager,
           self._proxy,
           _MN.GetFilesystemObjectPath,
           self._POOLNAME,
           'noname'
        )
        expected_rc = StratisdErrorsGen.get_object().FILESYSTEM_NOTFOUND
        self.assertEqual(rc, expected_rc)


class GetCacheTestCase(unittest.TestCase):
    """
    Test get_cache method when there is no pool.
    """

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        self._service = Service()
        self._service.setUp()
        time.sleep(1)
        self._proxy = get_object(TOP_OBJECT)

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    @unittest.skip("unimplemented")
    @unittest.expectedFailure
    def testNonExistingPool(self):
        """
        Currently, the error return code is DEV_NOTFOUND, it should
        be POOL_NOTFOUND
        """
        (_, rc, _) = checked_call(
           Manager,
           self._proxy,
           _MN.GetCacheObjectPath,
           'notapool'
        )
        expected_rc = StratisdErrorsGen.get_object().POOL_NOTFOUND
        self.assertEqual(rc, expected_rc)

    @unittest.skip("Unimplemented")
    def testNonExistingPool1(self):
        """
        Returns an error code, just the wrong one.
        """
        (_, rc, _) = checked_call(
           Manager,
           self._proxy,
           _MN.GetCacheObjectPath,
           'notapool'
        )
        ok_rc = StratisdErrorsGen.get_object().OK
        self.assertNotEqual(rc, ok_rc)


class GetCache1TestCase(unittest.TestCase):
    """
    Test get_cache method when there is a pool.
    """
    _POOLNAME = 'deadpool'

    def setUp(self):
        """
        Start the stratisd daemon with the simulator.
        """
        self._service = Service()
        self._service.setUp()
        time.sleep(1)
        self._proxy = get_object(TOP_OBJECT)
        Manager.callMethod(
           self._proxy,
           _MN.CreatePool,
           self._POOLNAME,
           0,
           False,
           [d.device_node for d in _device_list(_DEVICES, 1)]
        )

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    @unittest.skip("unimplemented")
    @unittest.expectedFailure
    def testException(self):
        """
        An error is returned if the pool does not exist.

        Unfortunately, it is the wrong error.
        """
        (_, rc, _) = checked_call(
           Manager,
           self._proxy,
           _MN.GetCacheObjectPath,
           'notapool'
        )
        expected_rc = StratisdErrorsGen.get_object().POOL_NOTFOUND
        self.assertEqual(rc, expected_rc)

    @unittest.skip("unimplemented")
    def testException1(self):
        """
        An error is returned if the pool does not exist.

        Aside from the error value, the results are correct.
        """
        (_, rc, _) = checked_call(
           Manager,
           self._proxy,
           _MN.GetCacheObjectPath,
           'notapool'
        )
        ok_rc = StratisdErrorsGen.get_object().OK
        self.assertNotEqual(rc, ok_rc)

    @unittest.skip("unimplemented")
    @unittest.expectedFailure
    def testExecution(self):
        """
        There should be success if the pool does exist.

        But, for some reason, there is not.
        """
        (result, rc, _) = checked_call(
           Manager,
           self._proxy,
           _MN.GetCacheObjectPath,
           self._POOLNAME
        )
        expected_rc = StratisdErrorsGen.get_object().OK
        self.assertEqual(rc, expected_rc)
        self.assertNotEqual(result, "")
