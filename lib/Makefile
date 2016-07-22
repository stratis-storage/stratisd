# Makefile
#
# Copyright (C) 2016 Red Hat, Inc.
#
# Todd Gill <tgill@redhat.com>, Gris Ge <fge@redhat.com>
#

LIBSTRATIS_VERSION=0.1.0
SONAME=$(LIBSTRATIS_VERSION)
DEVLIB = libstratis.so
LIBS = $(DEVLIB).$(SONAME)
LIBDEPS = 
PKGFILE = libstratis.pc
EXTRA_MAN_FILES = libstratis.h.3
HEADERS = libstratis/libstratis.h
OBJS = libstratis.o simulator.o
SHARED_FLAGS = -shared

CFLAGS += -g -I./ -I./stratis/ $(shell pkg-config --cflags glib-2.0) -fPIC -D_GNU_SOURCE \
		-DHAVE_SECURE_GETENV
LDFLAGS +=

all: $(LIBS)

$(LIBS): $(OBJS)
	$(CC) $(LDFLAGS) $(SHARED_FLAGS) \
	-Wl,-soname=$@ $(CFLAGS) -o $@ $(OBJS) $(LIBDEPS)
	ln -sf $@ $(DEVLIB)

install:
	$(INSTALL_PROGRAM) -m 755 $(LIBS) $(DESTDIR)$(syslibdir)/$(LIBS)
	$(INSTALL_PROGRAM) -m 644 -D \
		$(HEADERS) $(DESTDIR)/$(includedir)/$(HEADERS)
	ln -sf $(LIBS) $(DESTDIR)/$(syslibdir)/$(DEVLIB)
	$(INSTALL_PROGRAM) -m 644 -D \
		$(PKGFILE).in $(DESTDIR)/$(pkgconfdir)/$(PKGFILE)
	perl -i -pe 's|__VERSION__|$(LIBSTRATIS_VERSION)|g' \
		$(DESTDIR)/$(pkgconfdir)/$(PKGFILE)
	perl -i -pe 's|__LIBDIR__|$(syslibdir)|g' \
		$(DESTDIR)/$(pkgconfdir)/$(PKGFILE)
	perl -i -pe 's|__INCLUDEDIR__|$(includedir)|g' \
		$(DESTDIR)/$(pkgconfdir)/$(PKGFILE)
	@for file in docs/man/*.3.gz; do \
		$(INSTALL_PROGRAM) -m 644 -D \
			$$file \
			$(DESTDIR)/$(man3dir)/ || exit $?; \
	done

uninstall:
	rm -f $(DESTDIR)$(syslibdir)/$(LIBS)
	rm -f $(DESTDIR)$(includedir)/$(HEADERS)
	rm -f $(DESTDIR)/$(syslibdir)/$(DEVLIB)
	@for file in $(DESTDIR)/$(man3dir)/dmmp_*; do \
		rm $$file; \
	done
	rm -f $(DESTDIR)$(man3dir)/libstratis.h*

clean:
	rm -f core *.a *.o *.gz *.so *.so.*
	rm -f docs/man/*.3.gz
