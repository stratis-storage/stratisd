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

from stratisd_client_dbus._implementation import ManagerSpec

from .._constants import _DEVICES

from .._misc import checked_call
from .._misc import _device_list
from .._misc import Service

_MN = ManagerSpec.MethodNames

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
        self._errors = StratisdErrorsGen.get_object()

    def tearDown(self):
        """
        Stop the stratisd simulator and daemon.
        """
        self._service.tearDown()

    def testNonExistingPool(self):
        """
        An error code is returned if the pool does not exist.
        """
        (_, rc, _) = checked_call(
           Manager.GetPoolObjectPath(self._proxy, name="notapool"),
           ManagerSpec.OUTPUT_SIGS[_MN.GetPoolObjectPath]
        )
        self.assertEqual(rc, self._errors.POOL_NOTFOUND)


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
        self._errors = StratisdErrorsGen.get_object()
        Manager.CreatePool(
           self._proxy,
           name=self._POOLNAME,
           redundancy=0,
           force=False,
           devices=[d.device_node for d in _device_list(_DEVICES, 1)]
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
           Manager.GetPoolObjectPath(self._proxy, name=self._POOLNAME),
           ManagerSpec.OUTPUT_SIGS[_MN.GetPoolObjectPath],
        )
        self.assertEqual(rc, self._errors.OK)
        self.assertNotEqual(result, '')

    def testUnknownName(self):
        """
        Getting a non-existing pool should fail.
        """
        (_, rc, _) = checked_call(
           Manager.GetPoolObjectPath(self._proxy, name='nopool'),
           ManagerSpec.OUTPUT_SIGS[_MN.GetPoolObjectPath],
        )
        self.assertEqual(rc, self._errors.POOL_NOTFOUND)


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
        self._errors = StratisdErrorsGen.get_object()

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
           Manager.GetFilesystemObjectPath(
              self._proxy,
              pool_name='notapool',
              filesystem_name='noname'
           ),
           ManagerSpec.OUTPUT_SIGS[_MN.GetFilesystemObjectPath],
        )
        self.assertEqual(rc, self._errors.FILESYSTEM_NOTFOUND)


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
        self._errors = StratisdErrorsGen.get_object()
        Manager.CreatePool(
           self._proxy,
           name=self._POOLNAME,
           redundancy=0,
           force=False,
           devices=[d.device_node for d in _device_list(_DEVICES, 1)]
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
           Manager.GetFilesystemObjectPath(
              self._proxy,
              pool_name=self._POOLNAME,
              filesystem_name='noname'
           ),
           ManagerSpec.OUTPUT_SIGS[_MN.GetFilesystemObjectPath]
        )
        self.assertEqual(rc, self._errors.FILESYSTEM_NOTFOUND)


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
        self._errors = StratisdErrorsGen.get_object()
        (poolpath, _, _) = Manager.CreatePool(
           self._proxy,
           name=self._POOLNAME,
           redundancy=0,
           force=False,
           devices=[d.device_node for d in _device_list(_DEVICES, 1)]
        )
        Pool.CreateFilesystems(
           get_object(poolpath),
           specs=[(self._VOLNAME, '', 0)]
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
           Manager.GetFilesystemObjectPath(
              self._proxy,
              pool_name=self._POOLNAME,
              filesystem_name=self._VOLNAME
           ),
           ManagerSpec.OUTPUT_SIGS[_MN.GetFilesystemObjectPath]
        )
        self.assertEqual(rc, self._errors.OK)
        self.assertNotEqual(result, "")

    def testNonExistingVolume(self):
        """
        The volume does not exist.
        """
        (_, rc, _) = checked_call(
           Manager.GetFilesystemObjectPath(
              self._proxy,
              pool_name=self._POOLNAME,
              filesystem_name='noname'
           ),
           ManagerSpec.OUTPUT_SIGS[_MN.GetFilesystemObjectPath]
        )
        self.assertEqual(rc, self._errors.FILESYSTEM_NOTFOUND)


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
        self._errors = StratisdErrorsGen.get_object()

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
           Manager.GetCacheObjectPath(self._proxy, pool_name='notapool'),
           ManagerSpec.OUTPUT_SIGS[_MN.GetCacheObjectPath]
        )
        self.assertEqual(rc, self._errors.POOL_NOTFOUND)

    @unittest.skip("Unimplemented")
    def testNonExistingPool1(self):
        """
        Returns an error code, just the wrong one.
        """
        (_, rc, _) = checked_call(
           Manager.GetCacheObjectPath(self._proxy, pool_name='notapool'),
           ManagerSpec.OUTPUT_SIGS[_MN.GetCacheObjectPath]
        )
        self.assertNotEqual(rc, self._errors.OK)


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
        self._errors = StratisdErrorsGen.get_object()
        Manager.CreatePool(
           self._proxy,
           name=self._POOLNAME,
           redundancy=0,
           force=False,
           devices=[d.device_node for d in _device_list(_DEVICES, 1)]
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
           Manager.GetCacheObjectPath(self._proxy, pool_name='notapool'),
           ManagerSpec.OUTPUT_SIGS[_MN.GetCacheObjectPath]
        )
        self.assertEqual(rc, self._errors.POOL_NOTFOUND)

    @unittest.skip("unimplemented")
    def testException1(self):
        """
        An error is returned if the pool does not exist.

        Aside from the error value, the results are correct.
        """
        (_, rc, _) = checked_call(
           Manager.GetCacheObjectPath(self._proxy, pool_name='notapool'),
           ManagerSpec.OUTPUT_SIGS[_MN.GetCacheObjectPath]
        )
        self.assertNotEqual(rc, self._errors.OK)

    @unittest.skip("unimplemented")
    @unittest.expectedFailure
    def testExecution(self):
        """
        There should be success if the pool does exist.

        But, for some reason, there is not.
        """
        (result, rc, _) = checked_call(
           Manager.GetCacheObjectPath(self._proxy, pool_name=self._POOLNAME),
           ManagerSpec.OUTPUT_SIGS[_MN.GetCacheObjectPath]
        )
        self.assertEqual(rc, self._errors.OK)
        self.assertNotEqual(result, "")
