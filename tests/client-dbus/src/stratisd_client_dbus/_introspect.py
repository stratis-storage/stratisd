SPECS = {
    "org.freedesktop.DBus.ObjectManager": """
<interface name="org.freedesktop.DBus.ObjectManager">
    <method name="GetManagedObjects">
      <arg name="objpath_interfaces_and_properties" type="a{oa{sa{sv}}}" direction="out" />
    </method>
  </interface>
""",
    "org.storage.stratis3.Manager.r1": """
<interface name="org.storage.stratis3.Manager.r1">
    <method name="CreatePool">
      <arg name="name" type="s" direction="in" />
      <arg name="redundancy" type="(bq)" direction="in" />
      <arg name="devices" type="as" direction="in" />
      <arg name="key_desc" type="(bs)" direction="in" />
      <arg name="clevis_info" type="(b(ss))" direction="in" />
      <arg name="result" type="(b(oao))" direction="out" />
      <arg name="return_code" type="q" direction="out" />
      <arg name="return_string" type="s" direction="out" />
    </method>
    <method name="DestroyPool">
      <arg name="pool" type="o" direction="in" />
      <arg name="result" type="(bs)" direction="out" />
      <arg name="return_code" type="q" direction="out" />
      <arg name="return_string" type="s" direction="out" />
    </method>
    <method name="EngineStateReport">
      <arg name="result" type="s" direction="out" />
      <arg name="return_code" type="q" direction="out" />
      <arg name="return_string" type="s" direction="out" />
    </method>
    <method name="ListKeys">
      <arg name="result" type="as" direction="out" />
      <arg name="return_code" type="q" direction="out" />
      <arg name="return_string" type="s" direction="out" />
    </method>
    <method name="SetKey">
      <arg name="key_desc" type="s" direction="in" />
      <arg name="key_fd" type="h" direction="in" />
      <arg name="result" type="(bb)" direction="out" />
      <arg name="return_code" type="q" direction="out" />
      <arg name="return_string" type="s" direction="out" />
    </method>
    <method name="UnlockPool">
      <arg name="pool_uuid" type="s" direction="in" />
      <arg name="unlock_method" type="s" direction="in" />
      <arg name="result" type="(bas)" direction="out" />
      <arg name="return_code" type="q" direction="out" />
      <arg name="return_string" type="s" direction="out" />
    </method>
    <method name="UnsetKey">
      <arg name="key_desc" type="s" direction="in" />
      <arg name="result" type="b" direction="out" />
      <arg name="return_code" type="q" direction="out" />
      <arg name="return_string" type="s" direction="out" />
    </method>
    <property name="LockedPools" type="a{sa{sv}}" access="read" />
    <property name="Version" type="s" access="read">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const" />
    </property>
  </interface>
""",
    "org.storage.stratis3.Report.r1": """
<interface name="org.storage.stratis3.Report.r1">
    <method name="GetReport">
      <arg name="name" type="s" direction="in" />
      <arg name="result" type="s" direction="out" />
      <arg name="return_code" type="q" direction="out" />
      <arg name="return_string" type="s" direction="out" />
    </method>
  </interface>
""",
    "org.storage.stratis3.blockdev.r1": """
<interface name="org.storage.stratis3.blockdev.r1">
    <method name="SetUserInfo">
      <arg name="id" type="(bs)" direction="in" />
      <arg name="changed" type="(bs)" direction="out" />
      <arg name="return_code" type="q" direction="out" />
      <arg name="return_string" type="s" direction="out" />
    </method>
    <property name="Devnode" type="s" access="read">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const" />
    </property>
    <property name="HardwareInfo" type="(bs)" access="read">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const" />
    </property>
    <property name="InitializationTime" type="t" access="read">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const" />
    </property>
    <property name="PhysicalPath" type="s" access="read">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const" />
    </property>
    <property name="Pool" type="o" access="read">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const" />
    </property>
    <property name="Tier" type="q" access="read">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="false" />
    </property>
    <property name="TotalPhysicalSize" type="s" access="read" />
    <property name="UserInfo" type="(bs)" access="read">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="false" />
    </property>
    <property name="Uuid" type="s" access="read">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const" />
    </property>
  </interface>
""",
    "org.storage.stratis3.filesystem.r1": """
<interface name="org.storage.stratis3.filesystem.r1">
    <method name="SetName">
      <arg name="name" type="s" direction="in" />
      <arg name="result" type="(bs)" direction="out" />
      <arg name="return_code" type="q" direction="out" />
      <arg name="return_string" type="s" direction="out" />
    </method>
    <property name="Created" type="s" access="read">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const" />
    </property>
    <property name="Devnode" type="s" access="read">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="invalidates" />
    </property>
    <property name="Name" type="s" access="read" />
    <property name="Pool" type="o" access="read">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const" />
    </property>
    <property name="Size" type="s" access="read" />
    <property name="Used" type="(bs)" access="read" />
    <property name="Uuid" type="s" access="read">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const" />
    </property>
  </interface>
""",
    "org.storage.stratis3.pool.r1": """
<interface name="org.storage.stratis3.pool.r1">
    <method name="AddCacheDevs">
      <arg name="devices" type="as" direction="in" />
      <arg name="results" type="(bao)" direction="out" />
      <arg name="return_code" type="q" direction="out" />
      <arg name="return_string" type="s" direction="out" />
    </method>
    <method name="AddDataDevs">
      <arg name="devices" type="as" direction="in" />
      <arg name="results" type="(bao)" direction="out" />
      <arg name="return_code" type="q" direction="out" />
      <arg name="return_string" type="s" direction="out" />
    </method>
    <method name="BindClevis">
      <arg name="pin" type="s" direction="in" />
      <arg name="json" type="s" direction="in" />
      <arg name="results" type="b" direction="out" />
      <arg name="return_code" type="q" direction="out" />
      <arg name="return_string" type="s" direction="out" />
    </method>
    <method name="BindKeyring">
      <arg name="key_desc" type="s" direction="in" />
      <arg name="results" type="b" direction="out" />
      <arg name="return_code" type="q" direction="out" />
      <arg name="return_string" type="s" direction="out" />
    </method>
    <method name="CreateFilesystems">
      <arg name="specs" type="a(s(bs))" direction="in" />
      <arg name="results" type="(ba(os))" direction="out" />
      <arg name="return_code" type="q" direction="out" />
      <arg name="return_string" type="s" direction="out" />
    </method>
    <method name="DestroyFilesystems">
      <arg name="filesystems" type="ao" direction="in" />
      <arg name="results" type="(bas)" direction="out" />
      <arg name="return_code" type="q" direction="out" />
      <arg name="return_string" type="s" direction="out" />
    </method>
    <method name="InitCache">
      <arg name="devices" type="as" direction="in" />
      <arg name="results" type="(bao)" direction="out" />
      <arg name="return_code" type="q" direction="out" />
      <arg name="return_string" type="s" direction="out" />
    </method>
    <method name="RebindClevis">
      <arg name="results" type="b" direction="out" />
      <arg name="return_code" type="q" direction="out" />
      <arg name="return_string" type="s" direction="out" />
    </method>
    <method name="RebindKeyring">
      <arg name="key_desc" type="s" direction="in" />
      <arg name="results" type="b" direction="out" />
      <arg name="return_code" type="q" direction="out" />
      <arg name="return_string" type="s" direction="out" />
    </method>
    <method name="SetName">
      <arg name="name" type="s" direction="in" />
      <arg name="result" type="(bs)" direction="out" />
      <arg name="return_code" type="q" direction="out" />
      <arg name="return_string" type="s" direction="out" />
    </method>
    <method name="SnapshotFilesystem">
      <arg name="origin" type="o" direction="in" />
      <arg name="snapshot_name" type="s" direction="in" />
      <arg name="result" type="(bo)" direction="out" />
      <arg name="return_code" type="q" direction="out" />
      <arg name="return_string" type="s" direction="out" />
    </method>
    <method name="UnbindClevis">
      <arg name="results" type="b" direction="out" />
      <arg name="return_code" type="q" direction="out" />
      <arg name="return_string" type="s" direction="out" />
    </method>
    <method name="UnbindKeyring">
      <arg name="results" type="b" direction="out" />
      <arg name="return_code" type="q" direction="out" />
      <arg name="return_string" type="s" direction="out" />
    </method>
    <property name="AllocatedSize" type="s" access="read" />
    <property name="AvailableActions" type="s" access="read" />
    <property name="ClevisInfo" type="(b(b(ss)))" access="read" />
    <property name="Encrypted" type="b" access="read">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const" />
    </property>
    <property name="FsLimit" type="t" access="readwrite" />
    <property name="HasCache" type="b" access="read" />
    <property name="KeyDescription" type="(b(bs))" access="read" />
    <property name="Name" type="s" access="read" />
    <property name="NoAllocSpace" type="b" access="read" />
    <property name="Overprovisioning" type="b" access="readwrite" />
    <property name="TotalPhysicalSize" type="s" access="read" />
    <property name="TotalPhysicalUsed" type="(bs)" access="read" />
    <property name="Uuid" type="s" access="read">
      <annotation name="org.freedesktop.DBus.Property.EmitsChangedSignal" value="const" />
    </property>
  </interface>
""",
}
