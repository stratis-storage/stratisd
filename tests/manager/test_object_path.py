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

from .._misc import _device_list
from .._misc import Service


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
        (result, rc, message) = \
           Manager.callMethod(self._proxy, "GetPoolObjectPath", "notapool")
        expected_rc = StratisdErrorsGen.get_object().STRATIS_POOL_NOTFOUND
        self.assertEqual(rc, expected_rc)
        self.assertIsInstance(result, str)
        self.assertIsInstance(rc, int)
        self.assertIsInstance(message, str)


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
           "CreatePool",
           self._POOLNAME,
           [d.device_node for d in _device_list(_DEVICES, 1)],
           0
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
        (result, rc, message) = \
           Manager.callMethod(self._proxy, "GetPoolObjectPath", self._POOLNAME)
        self.assertEqual(rc, StratisdErrorsGen.get_object().STRATIS_OK)
        self.assertNotEqual(result, '')
        self.assertIsInstance(result, str)
        self.assertIsInstance(rc, int)
        self.assertIsInstance(message, str)

    def testUnknownName(self):
        """
        Getting a non-existing pool should fail.
        """
        (result, rc, message) = \
           Manager.callMethod(self._proxy, "GetPoolObjectPath", 'nopool')
        expected_rc = StratisdErrorsGen.get_object().STRATIS_POOL_NOTFOUND
        self.assertEqual(rc, expected_rc)
        self.assertIsInstance(result, str)
        self.assertIsInstance(rc, int)
        self.assertIsInstance(message, str)


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
        An exception is raised if the pool does not exist.
        """
        (result, rc, message) = Manager.callMethod(
           self._proxy,
           "GetVolumeObjectPath",
           'notapool',
           'noname'
        )
        expected_rc = StratisdErrorsGen.get_object().STRATIS_POOL_NOTFOUND
        self.assertEqual(rc, expected_rc)
        self.assertIsInstance(result, str)
        self.assertIsInstance(rc, int)
        self.assertIsInstance(message, str)


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
           "CreatePool",
           self._POOLNAME,
           [d.device_node for d in _device_list(_DEVICES, 1)],
           0
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
        (result, rc, message) = Manager.callMethod(
           self._proxy,
           "GetVolumeObjectPath",
           self._POOLNAME,
           'noname'
        )
        expected_rc = StratisdErrorsGen.get_object().STRATIS_VOLUME_NOTFOUND
        self.assertEqual(rc, expected_rc)
        self.assertIsInstance(result, str)
        self.assertIsInstance(rc, int)
        self.assertIsInstance(message, str)


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
           "CreatePool",
           self._POOLNAME,
           [d.device_node for d in _device_list(_DEVICES, 1)],
           0
        )
        (_, _, _) = Pool.callMethod(
           get_object(poolpath),
           "CreateVolumes",
           [(self._VOLNAME, '', '')]
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
        (result, rc, message) = Manager.callMethod(
           self._proxy,
           "GetVolumeObjectPath",
           self._POOLNAME,
           self._VOLNAME
        )
        expected_rc = StratisdErrorsGen.get_object().STRATIS_OK
        self.assertEqual(rc, expected_rc)
        self.assertNotEqual(result, "")
        self.assertIsInstance(result, str)
        self.assertIsInstance(rc, int)
        self.assertIsInstance(message, str)

    def testNonExistingVolume(self):
        """
        The volume does not exists.
        """
        (result, rc, message) = Manager.callMethod(
              self._proxy,
              "GetVolumeObjectPath",
              self._POOLNAME,
              'noname'
        )
        expected_rc = StratisdErrorsGen.get_object().STRATIS_VOLUME_NOTFOUND
        self.assertEqual(rc, expected_rc)
        self.assertIsInstance(result, str)
        self.assertIsInstance(rc, int)
        self.assertIsInstance(message, str)


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

    @unittest.expectedFailure
    def testNonExistingPool(self):
        """
        Currently, the error return code is STRATIS_DEV_NOTFOUND, it should
        be STRATIS_POOL_NOTFOUND
        """
        (result, rc, message) = \
           Manager.callMethod(self._proxy, "GetCacheObjectPath", 'notapool')
        expected_rc = StratisdErrorsGen.get_object().STRATIS_POOL_NOTFOUND
        self.assertEqual(rc, expected_rc)
        self.assertIsInstance(result, str)
        self.assertIsInstance(rc, int)
        self.assertIsInstance(message, str)

    def testNonExistingPool1(self):
        """
        Returns an error code, just the wrong one.
        """
        (result, rc, message) = \
           Manager.callMethod(self._proxy, "GetCacheObjectPath", 'notapool')
        ok_rc = StratisdErrorsGen.get_object().STRATIS_OK
        self.assertNotEqual(rc, ok_rc)
        self.assertIsInstance(result, str)
        self.assertIsInstance(rc, int)
        self.assertIsInstance(message, str)


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
           "CreatePool",
           self._POOLNAME,
           [d.device_node for d in _device_list(_DEVICES, 1)],
           0
        )

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    @unittest.expectedFailure
    def testException(self):
        """
        An error is returned if the pool does not exist.

        Unfortunately, it is the wrong error.
        """
        (result, rc, message) = \
           Manager.callMethod(self._proxy, "GetCacheObjectPath", 'notapool')
        expected_rc = StratisdErrorsGen.get_object().STRATIS_POOL_NOTFOUND
        self.assertEqual(rc, expected_rc)
        self.assertIsInstance(result, str)
        self.assertIsInstance(rc, int)
        self.assertIsInstance(message, str)

    def testException1(self):
        """
        An error is returned if the pool does not exist.

        Aside from the error value, the results are correct.
        """
        (result, rc, message) = \
           Manager.callMethod(self._proxy, "GetCacheObjectPath", 'notapool')
        ok_rc = StratisdErrorsGen.get_object().STRATIS_OK
        self.assertNotEqual(rc, ok_rc)
        self.assertIsInstance(result, str)
        self.assertIsInstance(rc, int)
        self.assertIsInstance(message, str)

    @unittest.expectedFailure
    def testExecution(self):
        """
        There should be success if the pool does exist.

        But, for some reason, there is not.
        """
        (result, rc, message) = \
           Manager.callMethod(self._proxy, "GetCacheObjectPath", self._POOLNAME)
        expected_rc = StratisdErrorsGen.get_object().STRATIS_OK
        self.assertEqual(rc, expected_rc)
        self.assertNotEqual(result, "")
        self.assertIsInstance(result, str)
        self.assertIsInstance(rc, int)
        self.assertIsInstance(message, str)
