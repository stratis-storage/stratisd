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
Representing stratisd contants.
"""

import abc

from ._connection import get_object

from ._constants import TOP_OBJECT

from ._implementation import Manager


class StratisdConstantsGen(abc.ABC):
    """
    Meta class for generating classes that define constants as class-level
    attributes.
    """
    # pylint: disable=too-few-public-methods

    _CLASSNAME = abc.abstractproperty(doc="the name of the class to construct")
    _METHOD = abc.abstractproperty(doc="dbus method")

    @classmethod
    def get_object(cls): # pragma: no cover
        """
        Read the available list from the bus.

        :return: class with class attributes for stratisd constants
        :rtype: type
        """
        values = cls._METHOD(get_object(TOP_OBJECT))

        def iterator():
            """
            An iterator over the fields in the class.
            """
            the_map = dict(values)
            for x in the_map:
                yield x

        fields = dict(values)
        fields['fields'] = iterator

        return type(cls._CLASSNAME, (object,), fields)


class StratisdErrorsGen(StratisdConstantsGen):
    """
    Simple class to provide access to published stratisd errors.
    """
    # pylint: disable=too-few-public-methods

    _CLASSNAME = 'StratisdErrors'
    _METHOD = Manager.Properties.ErrorValues


class StratisdRaidGen(StratisdConstantsGen):
    """
    Simple class to provide access to published stratisd errors.
    """
    # pylint: disable=too-few-public-methods

    _CLASSNAME = 'StratisdRedundancies'
    _METHOD = Manager.Properties.RedundancyValues
