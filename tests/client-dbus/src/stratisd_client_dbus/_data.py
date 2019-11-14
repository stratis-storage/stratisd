# Copyright 2017 Red Hat, Inc.
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
XML interface specifications.
"""

SPECS = {
    "org.freedesktop.DBus.ObjectManager": """
<interface name="org.freedesktop.DBus.ObjectManager">
<method name="GetManagedObjects">
<arg name="objpath_interfaces_and_properties" type="a{oa{sa{sv}}}" direction="out"/>
</method>
</interface>
""",
    "org.storage.stratis2.FetchProperties": """
<interface name="org.storage.stratis2.FetchProperties">
<method name="GetAllProperties">
<arg name="property_hash" type="a{s(bv)}" direction="out"/>
</method>
<method name="GetProperties">
<arg name="properties" type="as" direction="in"/>
<arg name="property_hash" type="a{s(bv)}" direction="out"/>
</method>
</interface>
""",
    "org.storage.stratis2.Manager": """
<interface name="org.storage.stratis2.Manager">
<method name="ConfigureSimulator">
<arg name="denominator" type="u" direction="in"/>
<arg name="return_code" type="q" direction="out"/>
<arg name="return_string" type="s" direction="out"/>
</method>
<method name="CreatePool">
<arg name="name" type="s" direction="in"/>
<arg name="redundancy" type="(bq)" direction="in"/>
<arg name="devices" type="as" direction="in"/>
<arg name="result" type="(b(oao))" direction="out"/>
<arg name="return_code" type="q" direction="out"/>
<arg name="return_string" type="s" direction="out"/>
</method>
<method name="DestroyPool">
<arg name="pool" type="o" direction="in"/>
<arg name="result" type="(bs)" direction="out"/>
<arg name="return_code" type="q" direction="out"/>
<arg name="return_string" type="s" direction="out"/>
</method>
<property name="Version" type="s" access="read">
<annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const"/>
</property>
</interface>
""",
    "org.storage.stratis2.Manager.r1": """
<interface name="org.storage.stratis2.Manager.r1">
<method name="ConfigureSimulator">
<arg name="denominator" type="u" direction="in"/>
<arg name="return_code" type="q" direction="out"/>
<arg name="return_string" type="s" direction="out"/>
</method>
<method name="CreatePool">
<arg name="name" type="s" direction="in"/>
<arg name="redundancy" type="(bq)" direction="in"/>
<arg name="devices" type="as" direction="in"/>
<arg name="keyfile_path" type="(bs)" direction="in"/>
<arg name="result" type="(b(oao))" direction="out"/>
<arg name="return_code" type="q" direction="out"/>
<arg name="return_string" type="s" direction="out"/>
</method>
<method name="DestroyPool">
<arg name="pool" type="o" direction="in"/>
<arg name="result" type="(bs)" direction="out"/>
<arg name="return_code" type="q" direction="out"/>
<arg name="return_string" type="s" direction="out"/>
</method>
<property name="Version" type="s" access="read">
<annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const"/>
</property>
</interface>
""",
    "org.storage.stratis2.pool": """
<interface name="org.storage.stratis2.pool">
<method name="AddCacheDevs">
<arg name="devices" type="as" direction="in"/>
<arg name="results" type="(bao)" direction="out"/>
<arg name="return_code" type="q" direction="out"/>
<arg name="return_string" type="s" direction="out"/>
</method>
<method name="AddDataDevs">
<arg name="devices" type="as" direction="in"/>
<arg name="results" type="(bao)" direction="out"/>
<arg name="return_code" type="q" direction="out"/>
<arg name="return_string" type="s" direction="out"/>
</method>
<method name="CreateFilesystems">
<arg name="specs" type="as" direction="in"/>
<arg name="results" type="(ba(os))" direction="out"/>
<arg name="return_code" type="q" direction="out"/>
<arg name="return_string" type="s" direction="out"/>
</method>
<method name="DestroyFilesystems">
<arg name="filesystems" type="ao" direction="in"/>
<arg name="results" type="(bas)" direction="out"/>
<arg name="return_code" type="q" direction="out"/>
<arg name="return_string" type="s" direction="out"/>
</method>
<method name="SetName">
<arg name="name" type="s" direction="in"/>
<arg name="result" type="(bs)" direction="out"/>
<arg name="return_code" type="q" direction="out"/>
<arg name="return_string" type="s" direction="out"/>
</method>
<method name="SnapshotFilesystem">
<arg name="origin" type="o" direction="in"/>
<arg name="snapshot_name" type="s" direction="in"/>
<arg name="result" type="(bo)" direction="out"/>
<arg name="return_code" type="q" direction="out"/>
<arg name="return_string" type="s" direction="out"/>
</method>
<property name="Name" type="s" access="read">
<annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="true"/>
</property>
<property name="Uuid" type="s" access="read">
<annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const"/>
</property>
</interface>
""",
    "org.storage.stratis2.pool.r1": """
<interface name="org.storage.stratis2.pool.r1">
<method name="InitCache">
<arg name="devices" type="as" direction="in"/>
<arg name="results" type="(bao)" direction="out"/>
<arg name="return_code" type="q" direction="out"/>
<arg name="return_string" type="s" direction="out"/>
</method>
<method name="AddCacheDevs">
<arg name="devices" type="as" direction="in"/>
<arg name="results" type="(bao)" direction="out"/>
<arg name="return_code" type="q" direction="out"/>
<arg name="return_string" type="s" direction="out"/>
</method>
<method name="AddDataDevs">
<arg name="devices" type="as" direction="in"/>
<arg name="results" type="(bao)" direction="out"/>
<arg name="return_code" type="q" direction="out"/>
<arg name="return_string" type="s" direction="out"/>
</method>
<method name="CreateFilesystems">
<arg name="specs" type="as" direction="in"/>
<arg name="results" type="(ba(os))" direction="out"/>
<arg name="return_code" type="q" direction="out"/>
<arg name="return_string" type="s" direction="out"/>
</method>
<method name="DestroyFilesystems">
<arg name="filesystems" type="ao" direction="in"/>
<arg name="results" type="(bas)" direction="out"/>
<arg name="return_code" type="q" direction="out"/>
<arg name="return_string" type="s" direction="out"/>
</method>
<method name="SetName">
<arg name="name" type="s" direction="in"/>
<arg name="result" type="(bs)" direction="out"/>
<arg name="return_code" type="q" direction="out"/>
<arg name="return_string" type="s" direction="out"/>
</method>
<method name="SnapshotFilesystem">
<arg name="origin" type="o" direction="in"/>
<arg name="snapshot_name" type="s" direction="in"/>
<arg name="result" type="(bo)" direction="out"/>
<arg name="return_code" type="q" direction="out"/>
<arg name="return_string" type="s" direction="out"/>
</method>
<property name="Name" type="s" access="read">
<annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="true"/>
</property>
<property name="Uuid" type="s" access="read">
<annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const"/>
</property>
</interface>
""",
    "org.storage.stratis2.filesystem": """
<interface name="org.storage.stratis2.filesystem">
<method name="SetName">
<arg name="name" type="s" direction="in"/>
<arg name="action" type="(bs)" direction="out"/>
<arg name="return_code" type="q" direction="out"/>
<arg name="return_string" type="s" direction="out"/>
</method>
<property name="Created" type="s" access="read">
<annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const"/>
</property>
<property name="Devnode" type="s" access="read">
<annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const"/>
</property>
<property name="Name" type="s" access="read">
<annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="true"/>
</property>
<property name="Pool" type="o" access="read">
<annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const"/>
</property>
<property name="Uuid" type="s" access="read">
<annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const"/>
</property>
</interface>
""",
    "org.storage.stratis2.blockdev": """
<interface name="org.storage.stratis2.blockdev">
<method name="SetUserInfo">
<arg name="id" type="(bs)" direction="in"/>
<arg name="changed" type="(bs)" direction="out"/>
<arg name="return_code" type="q" direction="out"/>
<arg name="return_string" type="s" direction="out"/>
</method>
<property name="Devnode" type="s" access="read">
<annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const"/>
</property>
<property name="HardwareInfo" type="(bs)" access="read">
<annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const"/>
</property>
<property name="InitializationTime" type="t" access="read">
<annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const"/>
</property>
<property name="Pool" type="o" access="read">
<annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const"/>
</property>
<property name="Tier" type="q" access="read">
<annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="false"/>
</property>
<property name="UserInfo" type="(bs)" access="read">
<annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="false"/>
</property>
<property name="Uuid" type="s" access="read">
<annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const"/>
</property>
</interface>
""",
}
