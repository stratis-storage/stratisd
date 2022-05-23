# Copyright 2021 Red Hat, Inc.
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
Test that predictions of space usage via different subcommands of
straits-predict-usage match.
"""

# isort: STDLIB
import json
import os
import subprocess
import unittest

# isort: THIRDPARTY
from justbytes import GiB, Range, TiB

_STRATIS_PREDICT_USAGE = os.environ["STRATIS_PREDICT_USAGE"]


def _call_predict_usage_pool(
    encrypted, device_sizes, *, fs_sizes=None, overprovision=True
):
    """
    Call stratis-predict-usage and return JSON result.

    :param bool encrypted: true if pool is to be encrypted
    :param device_sizes: list of sizes of devices for pool
    :type device_sizes: list of str
    :param fs_sizes: list of filesystem sizes
    :type fs_sizes: list of Range
    :param bool overprovision: whether it is allowed to overprovision the pool
    """
    with subprocess.Popen(
        [_STRATIS_PREDICT_USAGE, "pool"]
        + [f"--device-size={size.magnitude}" for size in device_sizes]
        + (
            []
            if fs_sizes is None
            else [(f"--filesystem-size={size.magnitude}") for size in fs_sizes]
        )
        + (["--encrypted"] if encrypted else [])
        + ([] if overprovision else ["--no-overprovision"]),
        stdout=subprocess.PIPE,
    ) as command:
        outs, errs = command.communicate()
        if command.returncode != 0:
            raise RuntimeError(
                f"Invocation of {_STRATIS_PREDICT_USAGE} returned an error: "
                f"{command.returncode,}, {errs}"
            )
        prediction = json.loads(outs)

    return prediction


def _call_predict_usage_filesystem(fs_sizes, overprovision):
    """
    Call stratis-predict-usage using filesystem subcommand.

    :param fs_specs: list of filesystem sizes
    :type fs_specs: list of Range
    :param bool overprovision: whether it is allowed to overprovision the pool
    """

    with subprocess.Popen(
        [_STRATIS_PREDICT_USAGE, "filesystem"]
        + [f"--filesystem-size={size.magnitude}" for size in fs_sizes]
        + ([] if overprovision else ["--no-overprovision"]),
        stdout=subprocess.PIPE,
    ) as command:
        outs, errs = command.communicate()
        if command.returncode != 0:
            raise RuntimeError(
                f"Invocation of {_STRATIS_PREDICT_USAGE} returned an error: "
                f"{command.returncode}, {errs}"
            )
        prediction = json.loads(outs)

    return prediction


class TestSpaceUsagePrediction(unittest.TestCase):
    """
    Test relations of filesystem prediction to pool prediction.
    """

    def test_prediction(self):
        """
        Verify that the prediction of space used by the filesystem subcommand
        is the same as the prediction obtained by computing over the results
        obtained by calling the pool subcommand with different arguments and
        taking the difference.
        """
        encrypted = False
        overprovisioned = True
        device_sizes = [Range(1, TiB)]
        fs_sizes = [Range(1, GiB)]

        pool_result_pre = _call_predict_usage_pool(
            encrypted, device_sizes, fs_sizes=None, overprovision=overprovisioned
        )
        pool_result_post = _call_predict_usage_pool(
            encrypted,
            device_sizes,
            fs_sizes=fs_sizes,
            overprovision=overprovisioned,
        )

        filesystem_result = _call_predict_usage_filesystem(
            fs_sizes, overprovision=overprovisioned
        )

        self.assertEqual(
            Range(pool_result_post["used"]) - Range(pool_result_pre["used"]),
            Range(filesystem_result["used"]),
        )

    def test_parsing(self):
        """
        Test some parsing behaviors.
        """

        with self.assertRaises(subprocess.CalledProcessError) as c_m:
            subprocess.run([_STRATIS_PREDICT_USAGE], check=True)

        exception = c_m.exception
        self.assertEqual(exception.returncode, 1)
