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
#include <semaphore.h>
#include <errno.h>
#include <pthread.h>
#include <glib.h>
#include <limits.h>
#include <string.h>
#include <libdmmp/libdmmp.h>
#include <systemd/sd-bus.h>

#include "libstratis.h"
#include "../lib/stratis-common.h"

GList * stratis_objects = NULL;


/*
 * create an array of strings suitable to pass to dbus
 */
static char **make_array(int size) {

    int i;
    char **array = (char **) malloc((size + 1) * sizeof (char *));

    if (array == NULL)
        return NULL;

    for (i = 0; i <= size; i++) {
        array[i] = NULL;
    }

    return array;
}



static char *get_long_str(long value) {

    const int n = snprintf(NULL, 0, "%ld", value);

    if (n < 0)
        return NULL;

    char *buffer = malloc(n + 1);

    if (buffer == NULL)
        return NULL;

    snprintf(buffer, n + 1, "%ld", value);

    return buffer;
}

char *make_object_name(const char *base_name, unsigned long name, char *object_type)
{
    char *obj_name = NULL;

    if (base_name == NULL || name < 0) {
        return NULL;
    }

    char *instance_name = get_long_str(name);

    if (instance_name == NULL)
        goto finished;

    int size = strlen(base_name) + strlen(instance_name) + strlen(object_type) + 2;

        obj_name = malloc(size);

        if (obj_name == NULL)
            return NULL;

    snprintf(obj_name, size, "%s/%s%s", base_name, object_type, instance_name);

finished:

    if (instance_name != NULL) {
        free(instance_name);
        instance_name = NULL;
    }

        return obj_name;
}


static int get_svolume_property(sd_bus *bus, const char *path,
		const char *interface, const char *property, sd_bus_message *reply,
		void *userdata, sd_bus_error *error) {

	svolume_t *svolume = userdata;

	if (strcmp(property, VOLUME_NAME) == 0)
		return sd_bus_message_append(reply, "s", stratis_svolume_get_name(svolume));

	if (strcmp(property, VOLUME_ID) == 0)
		return sd_bus_message_append(reply, "u", stratis_svolume_get_id(svolume));

	if (strcmp(property, VOLUME_MOUNT_POINT) == 0)
		return sd_bus_message_append(reply, "s", stratis_svolume_get_mount_point(svolume));

	// TODO deal with error
	return -1;
}

static int get_spool_property(sd_bus *bus, const char *path,
		const char *interface, const char *property, sd_bus_message *reply,
		void *userdata, sd_bus_error *error) {

	spool_t *spool = userdata;

	if (strcmp(property, POOL_NAME) == 0)
		return sd_bus_message_append(reply, "s", stratis_spool_get_name(spool));

	if (strcmp(property, POOL_ID) == 0)
		return sd_bus_message_append(reply, "u", stratis_spool_get_id(spool));

	// TODO deal with error
	return -1;
}
static int get_sdev_property(sd_bus *bus, const char *path,
		const char *interface, const char *property, sd_bus_message *reply,
		void *userdata, sd_bus_error *error) {

	sdev_t *sdev = userdata;

	if (strcmp(property, DEV_NAME) == 0)
		return sd_bus_message_append(reply, "s", stratis_sdev_get_name(sdev));

	if (strcmp(property, DEV_ID) == 0)
		return sd_bus_message_append(reply, "u", stratis_sdev_get_id(sdev));


	// TODO deal with error
	return -1;
}

static int something_handler(sd_bus_message *message,
			void *userdata,
			sd_bus_error *error) {
    struct context *c = userdata;
    const char *s;
    char *response = "ok";
    int r;

    r = sd_bus_message_read(message, "s", &s);

    r = sd_bus_reply_method_return(message, "s", response);

    return 1;

}

static const sd_bus_vtable spool_vtable[] = {
	SD_BUS_VTABLE_START(0),
	SD_BUS_PROPERTY(POOL_NAME, "s", get_spool_property, 0,
			SD_BUS_VTABLE_PROPERTY_CONST),
	SD_BUS_PROPERTY(POOL_ID, "s", get_spool_property, 0,
			SD_BUS_VTABLE_PROPERTY_CONST),
	SD_BUS_WRITABLE_PROPERTY("AutomaticIntegerProperty", "u", NULL, NULL,
		    offsetof(spool_t, size), 0),
	SD_BUS_VTABLE_END
};

static const sd_bus_vtable svolume_vtable[] = {
	SD_BUS_VTABLE_START(0),
	SD_BUS_PROPERTY(VOLUME_NAME, "s", get_svolume_property, 0,
			SD_BUS_VTABLE_PROPERTY_CONST),
	SD_BUS_PROPERTY(VOLUME_ID, "s", get_svolume_property, 0,
			SD_BUS_VTABLE_PROPERTY_CONST),
			/*
	SD_BUS_PROPERTY(VOLUME_MOUNT_POINT, "s", get_svolume_property, 0,
			SD_BUS_VTABLE_PROPERTY_CONST),
    SD_BUS_WRITABLE_PROPERTY("AutomaticIntegerProperty", "u", NULL, NULL,
    		offsetof(svolume_t, size), 0),
	SD_BUS_WRITABLE_PROPERTY("AutomaticStringProperty", "s", NULL, NULL,
		    offsetof(svolume_t, name), 0),
	SD_BUS_WRITABLE_PROPERTY("AutomaticStringProperty", "s", NULL, NULL,
			offsetof(svolume_t, mount_point), 0),
	SD_BUS_METHOD("AlterSomething", "s", "s", something_handler, 0), */
	SD_BUS_VTABLE_END
};

static const sd_bus_vtable sdev_vtable[] = {
	SD_BUS_VTABLE_START(0),
	SD_BUS_PROPERTY(DEV_NAME, "s", get_sdev_property, 0,
			SD_BUS_VTABLE_PROPERTY_CONST),
	SD_BUS_PROPERTY(DEV_ID, "s", get_sdev_property, 0,
			SD_BUS_VTABLE_PROPERTY_CONST),
	SD_BUS_PROPERTY(DEV_TYPE, "s", get_sdev_property, 0,
			SD_BUS_VTABLE_PROPERTY_CONST),
	SD_BUS_VTABLE_END
};

int sync_stratis(sd_bus *bus, sd_bus_slot *slot) {
	int rc = EXIT_SUCCESS;
	int dbus_rc;
	int spool_list_size = 0, svolume_list_size = 0, sdev_list_size = 0;
	char spool_name[256], svolume_name[256], sdev_name[256];

	int i, j, k;
	spool_t *spool;
	svolume_t *svolume;
	sdev_t *sdev;
	svolume_list_t *svolume_list;
	sdev_list_t *sdev_list;

	spool_list_t *spool_list;

	rc = stratis_spool_get_list(&spool_list);
	if (rc != STRATIS_OK) {
		fprintf(stderr, "Failed stratis_spool_get_list()\n");
		goto out;
	}

	rc =  stratis_spool_list_size(spool_list, &spool_list_size);
	if (rc != STRATIS_OK) {
		fprintf(stderr, "Failed stratis_spool_get_list()\n");
		goto out;
	}
	for (i = 0; i < spool_list_size; i++) {

		rc = stratis_spool_list_nth(spool_list,
				&spool,
				i);
		if (rc != STRATIS_OK) {
			fprintf(stderr, "Failed stratis_spool_get_list()\n");
			goto out;
		}
		snprintf(spool_name, 256, "%s/%s", STRATIS_BASE_PATH,
				stratis_spool_get_name(spool));

		dbus_rc = sd_bus_add_object_vtable(bus,
								&slot, spool_name,
								STRATIS_POOL_BASE_INTERFACE,
								spool_vtable, spool);
		if (dbus_rc < 0) {
			fprintf(stderr, "Failed to connect to system bus: %s\n", strerror(dbus_rc));
			goto out;
		}
		rc = stratis_spool_get_volume_list(spool, &svolume_list);
		if (rc != STRATIS_OK) {
			fprintf(stderr, "Failed stratis_spool_get_list()\n");
			goto out;
		}
		rc =  stratis_svolume_list_size(svolume_list, &svolume_list_size);
		if (rc != STRATIS_OK) {
			fprintf(stderr, "Failed stratis_spool_get_list()\n");
			goto out;
		}
		for (j = 0; j < svolume_list_size; j++) {

			rc = stratis_svolume_list_nth(svolume_list,
					&svolume,
					j);
			if (rc != STRATIS_OK) {
				fprintf(stderr, "Failed stratis_spool_get_list()\n");
				goto out;
			}
			snprintf(svolume_name, 256, "%s/%s", STRATIS_BASE_PATH,
					stratis_svolume_get_name(svolume));

			dbus_rc = sd_bus_add_object_vtable(bus,
									&slot, svolume_name,
									STRATIS_VOLUME_BASE_INTERFACE,
									svolume_vtable, svolume);
		}

		rc = stratis_spool_get_dev_list(spool, &sdev_list);
		if (rc != STRATIS_OK) {
			fprintf(stderr, "Failed stratis_spool_get_list()\n");
			goto out;
		}
		rc =  stratis_sdev_list_size(sdev_list, &sdev_list_size);
		if (rc != STRATIS_OK) {
			fprintf(stderr, "Failed stratis_spool_get_list()\n");
			goto out;
		}
		for (k = 0; k < sdev_list_size; k++) {

			rc = stratis_sdev_list_nth(sdev_list,
					&sdev,
					k);
			if (rc != STRATIS_OK) {
				fprintf(stderr, "Failed stratis_spool_get_list()\n");
				goto out;
			}
			snprintf(sdev_name, 256, "%s/%s", STRATIS_BASE_PATH,
					stratis_sdev_get_name(sdev));

			dbus_rc = sd_bus_add_object_vtable(bus,
									&slot, sdev_name,
									STRATIS_DEV_BASE_INTERFACE,
									sdev_vtable, sdev);

		}
	}

out:

	return rc;
}


/*
 * This is the main loop of the d-bus service.  It won't exit until
 * quit_dbus_main_loop() is called.
 *
 * It is should be invoked as the startup function of a thread or the caller
 * should not expect it to return.
 */
void * stratis_main_loop(void * ap) {
	sd_bus_slot *slot = NULL;
	sd_bus *bus = NULL;
	int r;

	/* Connect to the user bus this time */
	/*
	 * TODO Connect to the system bus with sd_bus_open_system();
	 *
	 * for now, open the session bus so we don't need a .conf file.
	 *
	 */
	r = sd_bus_open_user(&bus);
	if (r < 0) {
		fprintf(stderr, "Failed to connect to system bus: %s\n", strerror(-r));
		goto finish;
	}

	populate_simulator_test_data();

	sync_stratis(bus, slot);

	/* Take a well-known service name so that clients can find us */
	r = sd_bus_request_name(bus, STRATIS_BASE_INTERFACE, 0);

	if (r < 0) {
		fprintf(stderr, "Failed to acquire service name: %s\n", strerror(-r));
		goto finish;
	}

	for (;;) {
		/* Process requests */
		r = sd_bus_process(bus, NULL);
		if (r < 0) {
			fprintf(stderr, "Failed to process bus: %s\n", strerror(-r));
			goto finish;
		}
		if (r > 0) /* we processed a request, try to process another one, right-away */
			continue;

		/* Wait for the next request to process */
		r = sd_bus_wait(bus, (uint64_t) - 1);
		if (r < 0) {
			fprintf(stderr, "Failed to wait on bus: %s\n", strerror(-r));
			goto finish;
		}
	}

	finish: sd_bus_slot_unref(slot);
	sd_bus_unref(bus);

	return NULL;
}
