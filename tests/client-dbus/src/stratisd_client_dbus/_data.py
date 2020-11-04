SPECS = {
    "org.freedesktop.DBus.ObjectManager": """
<interface name="org.freedesktop.DBus.ObjectManager">
    <method name="GetManagedObjects">
      <arg direction="out" name="objpath_interfaces_and_properties" type="a{oa{sa{sv}}}" />
    </method>
  </interface>
""",
    "org.storage.stratis2.FetchProperties.r2": """
<interface name="org.storage.stratis2.FetchProperties.r2">
    <method name="GetAllProperties">
      <arg direction="out" name="results" type="a{s(bv)}" />
    </method>
    <method name="GetProperties">
      <arg direction="in" name="properties" type="as" />
      <arg direction="out" name="results" type="a{s(bv)}" />
    </method>
  </interface>
""",
    "org.storage.stratis2.Manager.r3": """
<interface name="org.storage.stratis2.Manager.r2">
    <method name="ConfigureSimulator">
      <arg direction="in" name="denominator" type="u" />
      <arg direction="out" name="return_code" type="q" />
      <arg direction="out" name="return_string" type="s" />
    </method>
    <method name="CreatePool">
      <arg direction="in" name="name" type="s" />
      <arg direction="in" name="redundancy" type="(bq)" />
      <arg direction="in" name="devices" type="as" />
      <arg direction="in" name="key_desc" type="(bs)" />
      <arg direction="out" name="result" type="(b(oao))" />
      <arg direction="out" name="return_code" type="q" />
      <arg direction="out" name="return_string" type="s" />
    </method>
    <method name="DestroyPool">
      <arg direction="in" name="pool" type="o" />
      <arg direction="out" name="result" type="(bs)" />
      <arg direction="out" name="return_code" type="q" />
      <arg direction="out" name="return_string" type="s" />
    </method>
    <method name="SetKey">
      <arg direction="in" name="key_desc" type="s" />
      <arg direction="in" name="key_fd" type="h" />
      <arg direction="in" name="interactive" type="b" />
      <arg direction="out" name="result" type="(bb)" />
      <arg direction="out" name="return_code" type="q" />
      <arg direction="out" name="return_string" type="s" />
    </method>
    <method name="UnlockPool">
      <arg direction="in" name="pool_uuid" type="s" />
      <arg direction="in" name="unlock_method" type="s" />
      <arg direction="out" name="result" type="(bas)" />
      <arg direction="out" name="return_code" type="q" />
      <arg direction="out" name="return_string" type="s" />
    </method>
    <method name="UnsetKey">
      <arg direction="in" name="key_desc" type="s" />
      <arg direction="out" name="result" type="b" />
      <arg direction="out" name="return_code" type="q" />
      <arg direction="out" name="return_string" type="s" />
    </method>
    <property access="read" name="Version" type="s">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const" />
    </property>
  </interface>
""",
    "org.storage.stratis2.Report.r1": """
<interface name="org.storage.stratis2.Report.r1">
    <method name="GetReport">
      <arg direction="in" name="name" type="s" />
      <arg direction="out" name="result" type="s" />
      <arg direction="out" name="return_code" type="q" />
      <arg direction="out" name="return_string" type="s" />
    </method>
  </interface>
""",
    "org.storage.stratis2.blockdev.r2": """
<interface name="org.storage.stratis2.blockdev.r2">
    <method name="SetUserInfo">
      <arg direction="in" name="id" type="(bs)" />
      <arg direction="out" name="changed" type="(bs)" />
      <arg direction="out" name="return_code" type="q" />
      <arg direction="out" name="return_string" type="s" />
    </method>
    <property access="read" name="Devnode" type="s">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const" />
    </property>
    <property access="read" name="HardwareInfo" type="(bs)">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const" />
    </property>
    <property access="read" name="InitializationTime" type="t">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const" />
    </property>
    <property access="read" name="PhysicalPath" type="s">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const" />
    </property>
    <property access="read" name="Pool" type="o">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const" />
    </property>
    <property access="read" name="Tier" type="q">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="false" />
    </property>
    <property access="read" name="UserInfo" type="(bs)">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="false" />
    </property>
    <property access="read" name="Uuid" type="s">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const" />
    </property>
  </interface>
""",
    "org.storage.stratis2.filesystem": """
<interface name="org.storage.stratis2.filesystem">
    <method name="SetName">
      <arg direction="in" name="name" type="s" />
      <arg direction="out" name="result" type="(bs)" />
      <arg direction="out" name="return_code" type="q" />
      <arg direction="out" name="return_string" type="s" />
    </method>
    <property access="read" name="Created" type="s">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const" />
    </property>
    <property access="read" name="Devnode" type="s">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const" />
    </property>
    <property access="read" name="Name" type="s" />
    <property access="read" name="Pool" type="o">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const" />
    </property>
    <property access="read" name="Uuid" type="s">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const" />
    </property>
  </interface>
""",
    "org.storage.stratis2.pool.r3": """
<interface name="org.storage.stratis2.pool.r1">
    <method name="AddCacheDevs">
      <arg direction="in" name="devices" type="as" />
      <arg direction="out" name="results" type="(bao)" />
      <arg direction="out" name="return_code" type="q" />
      <arg direction="out" name="return_string" type="s" />
    </method>
    <method name="AddDataDevs">
      <arg direction="in" name="devices" type="as" />
      <arg direction="out" name="results" type="(bao)" />
      <arg direction="out" name="return_code" type="q" />
      <arg direction="out" name="return_string" type="s" />
    </method>
    <method name="Bind">
      <arg direction="in" name="pin" type="s" />
      <arg direction="in" name="json" type="s" />
      <arg direction="out" name="results" type="b" />
      <arg direction="out" name="return_code" type="q" />
      <arg direction="out" name="return_string" type="s" />
    </method>
    <method name="CreateFilesystems">
      <arg direction="in" name="specs" type="as" />
      <arg direction="out" name="results" type="(ba(os))" />
      <arg direction="out" name="return_code" type="q" />
      <arg direction="out" name="return_string" type="s" />
    </method>
    <method name="DestroyFilesystems">
      <arg direction="in" name="filesystems" type="ao" />
      <arg direction="out" name="results" type="(bas)" />
      <arg direction="out" name="return_code" type="q" />
      <arg direction="out" name="return_string" type="s" />
    </method>
    <method name="InitCache">
      <arg direction="in" name="devices" type="as" />
      <arg direction="out" name="results" type="(bao)" />
      <arg direction="out" name="return_code" type="q" />
      <arg direction="out" name="return_string" type="s" />
    </method>
    <method name="SetName">
      <arg direction="in" name="name" type="s" />
      <arg direction="out" name="result" type="(bs)" />
      <arg direction="out" name="return_code" type="q" />
      <arg direction="out" name="return_string" type="s" />
    </method>
    <method name="SnapshotFilesystem">
      <arg direction="in" name="origin" type="o" />
      <arg direction="in" name="snapshot_name" type="s" />
      <arg direction="out" name="result" type="(bo)" />
      <arg direction="out" name="return_code" type="q" />
      <arg direction="out" name="return_string" type="s" />
    </method>
    <method name="Unbind">
      <arg direction="out" name="results" type="b" />
      <arg direction="out" name="return_code" type="q" />
      <arg direction="out" name="return_string" type="s" />
    </method>
    <property access="read" name="Encrypted" type="b">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const" />
    </property>
    <property access="read" name="Name" type="s" />
    <property access="read" name="Uuid" type="s">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const" />
    </property>
  </interface>
""",
}
