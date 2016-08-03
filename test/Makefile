#
# Copyright (C) 2016 Red Hat, Inc.
#
# This program is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 3 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program.  If not, see <http://www.gnu.org/licenses/>.
#
# Author: Todd Gill <tgill@redhat.com>
#

EXEC = stratis_test 

CC = gcc
#
# basic flags setting
#
CFLAGS += -g -fPIC $(shell pkg-config --cflags glib-2.0) -I../lib/stratis \
	-I../lib
LDFLAGS +=  -L../lib -ldl -lpthread $(shell pkg-config --libs glib-2.0 libsystemd) \
	-lstratis

%.o:	%.c
	$(CC) $(CFLAGS) -c -o $@ $<

#
# object files
#
OBJS = test.o 

#
# directives
#

all : $(EXEC)

$(EXEC): $(OBJS)
	$(CC) $(CFLAGS) $(OBJS) $(LDFLAGS) -o $(EXEC)

clean:
	rm -f core *.o $(EXEC)

