/*
 * Copyright (C) 2016 Red Hat, Inc.
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <http://www.gnu.org/licenses/>.
 *
 * Author: Todd Gill <tgill@redhat.com>
 */
#include <stdio.h>
#include <stdlib.h>
#include <systemd/sd-bus.h>

#include "stratis-common.h"
#include "libstratis.h"
#include "test.h"

#define TEST_DEV_COUNT 20
#define TEST_POOL_COUNT 10
#define TEST_VOLUME_COUNT 5


int main(int argc, char **argv) {
	sd_bus_error error = SD_BUS_ERROR_NULL;
	sd_bus_message *m = NULL;
	sd_bus *bus = NULL;
	const char *path;
	int r;

	/* Connect to the system bus */
	r = sd_bus_open_user(&bus);
	if (r < 0) {
		fprintf(stderr, "Failed to connect to system bus: %s\n", strerror(-r));
		goto finish;
	}

	/* Issue the method call and store the respons message in m */
	r = sd_bus_call_method(bus,
					STRATIS_BASE_SERVICE, /* service to contact */
					STRATIS_BASE_PATH, /* object path */
					STRATIS_MANAGER_INTERFACE, /* interface name */
					"CreatePool", /* method name */
					&error, /* object to return error in */
					&m, /* return message on success */
					"ss", /* input signature */
					"pool.name", /* first argument */
					"raid5"); /* second argument */

	if (r < 0) {
		fprintf(stderr, "Failed to issue method call: %s\n", error.message);
		goto finish;
	}

	/* Parse the response message */
	r = sd_bus_message_read(m, "o", &path);
	if (r < 0) {
		fprintf(stderr, "Failed to parse response message: %s\n", strerror(-r));
		goto finish;
	}

	printf("Queued service job as %s.\n", path);

	finish: sd_bus_error_free(&error);
	sd_bus_message_unref(m);
	sd_bus_unref(bus);

	return r < 0 ? EXIT_FAILURE : EXIT_SUCCESS;
}
