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


class StratisdConstants(object):
    """
    Simple class to provide access to published stratisd constants.
    """

    @staticmethod
    def parse_error_list(error_list):
        """
        Parse the list of stratisd errors.

        :param error_list: list of errors published by stratisd
        :type error_list: Array of String * `a * String

        :returns: the values and the descriptions attached to the errors
        :rtype: (dict of String * `a) * (dict of `a * String)
        """

        values = dict()
        descriptions = dict()
        for (key, value, desc) in error_list:
            values[key] = value
            descriptions[value] = desc
        return (values, descriptions)

    @staticmethod
    def build_class(classname, values):
        """
        Build a StratisdErrors class with a bunch of class attributes which
        represent the stratisd errors.

        :param str classname: the name of the class to construct
        :param values: the values for the attributes
        :type values: dict of String * Int32
        :rtype: type
        :returns: StratisdError class
        """
        values['FIELDS'] = [x for x in values.keys()]
        return type(classname, (object,), values)

    @staticmethod
    def get_class(classname, error_list):
        """
        Get a class from ``error_list``.

        :param str classname: the name of the class to construct
        :param error_list: list of errors published by stratisd
        :type error_list: Array of String * `a * String

        :returns: the class which supports a mapping from error codes to ints
        :rtype: type
        """
        (values, _) = StratisdConstants.parse_error_list(error_list)
        return StratisdConstants.build_class(classname, values)


class StratisdConstantsGen(abc.ABC):
    """
    Meta class for generating classes that define constants as class-level
    attributes.
    """
    # pylint: disable=too-few-public-methods

    _CLASSNAME = abc.abstractproperty(doc="the name of the class to construct")
    _METHODNAME = abc.abstractproperty(doc="dbus method name")

    @classmethod
    def get_object(cls):
        """
        Read the available list from the bus.

        :return: class with class attributes for stratisd constants
        :rtype: type
        """
        values = getattr(Manager(get_object(TOP_OBJECT)), cls._METHODNAME)()
        return StratisdConstants.get_class(cls._CLASSNAME, values)


class StratisdErrorsGen(StratisdConstantsGen):
    """
    Simple class to provide access to published stratisd errors.
    """
    # pylint: disable=too-few-public-methods

    _CLASSNAME = 'StratisdErrors'
    _METHODNAME = 'GetErrorCodes'

class StratisdRaidGen(StratisdConstantsGen):
    """
    Simple class to provide access to published stratisd raid levels.
    """
    # pylint: disable=too-few-public-methods

    _CLASSNAME = 'StratisdRaidLevels'
    _METHODNAME = 'GetRaidLevels'
