%global build_timestamp %{lua: print(os.date("%Y%m%d%H%M"))}
%global oname stratisd

Name:           stratisd-prerelease
License:        MPL 2.0
Group:          System Environment/Libraries
Summary:        A daemon that manages block devices to create filesystems
Version:        1.0.99
Release:        1.%{build_timestamp}%{?dist}
URL:            https://github.com/stratis-storage/stratisd
Source0:        %{url}/archive/master.tar.gz
BuildRequires:  rust cargo dbus-devel systemd-devel
BuildRequires:  %{_bindir}/a2x
BuildRequires:  systemd

%{?systemd_requires}
Requires:       xfsprogs
Requires:       device-mapper-persistent-data

Conflicts: stratisd

%description
A daemon that manages block devices to create filesystems.

%prep
%autosetup -n %{oname}-master
# %cargo_prep

%build
# %cargo_build
# %cargo_build -n
# cargo build # --release
a2x -f manpage docs/stratisd.txt

%install
# %cargo_install
# Daemon should be really private
mkdir -p %{buildroot}%{_libexecdir}
cargo install --root %{buildroot} --debug
mv %{buildroot}/bin/stratisd %{buildroot}%{_libexecdir}/stratisd

# %cargo_install -n
# Init version should be in sbin
mkdir -p %{buildroot}%{_sbindir}

cargo install --root %{buildroot} --debug --no-default-features
mv %{buildroot}/bin/stratisd %{buildroot}%{_sbindir}/stratisd-init

# cargo install cruft.
rm %{buildroot}/.crates.toml

%{__install} -Dpm0644 -t %{buildroot}%{_datadir}/dbus-1/system.d stratisd.conf
%{__install} -Dpm0644 -t %{buildroot}%{_mandir}/man8 docs/stratisd.8
%{__install} -Dpm0644 -t %{buildroot}%{_unitdir} stratisd.service

%post
%systemd_post stratisd.service

%preun
%systemd_preun stratisd.service

%postun
%systemd_postun_with_restart stratisd.service

%files
%license LICENSE
%doc README.md
%{_libexecdir}/stratisd
%{_sbindir}/stratisd-init
%dir %{_datadir}/dbus-1
%{_datadir}/dbus-1/system.d/stratisd.conf
%{_mandir}/man8/stratisd.8*
%{_unitdir}/stratisd.service

%changelog
* Wed Oct 3 2018 Andy Grover <agrover@redhat.com> - 1.0.0-1
- This is not relevant for automated COPR-built packages.
