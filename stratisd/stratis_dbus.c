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
#include <syslog.h>
#include <systemd/sd-bus.h>

#include "libstratis.h"
#include "../lib/stratis-common.h"

static sd_bus *bus = NULL;

static int sync_spool(spool_t *spool);
static int sync_volume(svolume_t *volume, spool_t *spool);

static int find_spool(char *name, spool_t **spool) {
	int r = STRATIS_OK;
	spool_table_t *spool_list;

	r = stratis_spool_get_list(&spool_list);
	if (r != STRATIS_OK) {
		r = STRATIS_NOTFOUND;
		goto out;
	}

	r = stratis_spool_table_find(spool_list, spool, name);

out:
	return r;
}

static int find_volume(char *name, spool_t *spool,
			svolume_t **volume) {
	int r = STRATIS_OK;
	svolume_table_t *svolume_list;


	r = stratis_spool_get_volume_list(spool, &svolume_list);

	if (r != STRATIS_OK) {
		r = STRATIS_NOTFOUND;
		goto out;
	}

	r = stratis_svolume_table_find(svolume_list, volume, name);

	out: return r;
}

static int get_sdev_property(sd_bus *bus, const char *path,
        const char *interface, const char *property, sd_bus_message *reply,
        void *userdata, sd_bus_error *error) {

	sdev_t *sdev = userdata;

	if (strcmp(property, DEV_NAME) == 0)
		return sd_bus_message_append(reply, "s",
		        stratis_sdev_get_name(sdev));

	if (strcmp(property, DEV_ID) == 0)
		return sd_bus_message_append(reply, "u",
		        stratis_sdev_get_id(sdev));

	// TODO deal with error
	return -1;
}

static int get_svolume_property(sd_bus *bus, const char *path,
        const char *interface, const char *property, sd_bus_message *reply,
        void *userdata, sd_bus_error *error) {

	svolume_t *svolume = userdata;

	if (strcmp(property, VOLUME_NAME) == 0)
		return sd_bus_message_append(reply, "s",
		        stratis_svolume_get_name(svolume));

	if (strcmp(property, VOLUME_ID) == 0)
		return sd_bus_message_append(reply, "u",
		        stratis_svolume_get_id(svolume));

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

static int property_get_version(sd_bus *bus, const char *path,
        const char *interface, const char *property, sd_bus_message *reply,
        void *userdata, sd_bus_error *error) {

	return sd_bus_message_append(reply, "s", STRATIS_VERSION);
}

static int property_get_log_level(sd_bus *bus, const char *path,
        const char *interface, const char *property, sd_bus_message *reply,
        void *userdata, sd_bus_error *error) {

	return sd_bus_message_append(reply, "s", "LOGLEVELX");
}

static int property_set_log_level(sd_bus *bus, const char *path,
        const char *interface, const char *property, sd_bus_message *value,
        void *userdata, sd_bus_error *error) {

	const char *t;
	int rc;

	rc = sd_bus_message_read(value, "s", &t);
	if (rc < 0)
		return rc;

	/* TODO set log level here */

	return rc;
}

static int get_error_codes(sd_bus_message *message, void *userdata,
        sd_bus_error *error) {
	sd_bus_message *reply = NULL;
	int rc;

	rc = sd_bus_message_new_method_return(message, &reply);
	if (rc < 0)
		return rc;

	rc = sd_bus_message_open_container(reply, 'a', "(is)");
	if (rc < 0)
		goto out;

	for (int i = STRATIS_OK; i < STRATIS_ERROR_MAX; i++) {
		sd_bus_message_append(reply, "(is)", i, stratis_get_user_message(i));
	}

	sd_bus_message_close_container(reply);
	return sd_bus_send(NULL, reply, NULL);

out:
	return rc;
}

static void
iterate_spools (gpointer key, gpointer value, gpointer user_data)
{
	sd_bus_message *reply = user_data;
	char *name = key;

	sd_bus_message_append(reply, "s", key);

}

static int list_pools(sd_bus_message *message, void *userdata,
        sd_bus_error *error) {
	sd_bus_message *reply = NULL;
	spool_table_t *spool_list;
	spool_t *spool;
	int spool_list_size = 0;
	int rc, i;

	rc = sd_bus_message_new_method_return(message, &reply);
	if (rc < 0)
		return rc;

	rc = sd_bus_message_open_container(reply, 'a', "s");
	if (rc < 0)
		goto out;

	rc = stratis_spool_get_list(&spool_list);
	if (rc != STRATIS_OK) {
		fprintf(stderr, "Failed stratis_spool_get_list()\n");
		goto out;
	}

	rc = stratis_spool_list_size(spool_list, &spool_list_size);
	if (rc != STRATIS_OK) {

		if (spool_list == NULL) {
			rc = STRATIS_OK;
		}
		goto out;
	}

	g_hash_table_foreach(spool_list->table, iterate_spools, reply);

out:
	sd_bus_message_close_container(reply);

	sd_bus_message_append(reply, "is", rc, stratis_get_user_message(rc));

	return sd_bus_send(NULL, reply, NULL);

}

static void
iterate_sdevs (gpointer key, gpointer value, gpointer user_data)
{
	sd_bus_message *reply = user_data;
	char *sdev = key;

	sd_bus_message_append(reply, "s", sdev);

}

static int list_pool_devs(sd_bus_message *message, void *userdata,
        sd_bus_error *error) {
	sdev_table_t *sdev_list;
	spool_t *spool = (spool_t *) userdata;
	char *sdev = NULL;
	sd_bus_message *reply = NULL;
	int dev_list_size = 0;
	int rc, i;

	rc = sd_bus_message_new_method_return(message, &reply);
	if (rc < 0)
		return rc;

	rc = sd_bus_message_open_container(reply, 'a', "s");
	if (rc < 0)
		goto out;

	rc = stratis_spool_get_dev_table(spool, &sdev_list);
	if (rc != STRATIS_OK) {
		fprintf(stderr, "Failed stratis_svolume_get_list()\n");
		goto out;
	}

	g_hash_table_foreach(sdev_list->table, iterate_sdevs, reply);

	rc = sd_bus_message_close_container(reply);
	if (rc < 0)
		return rc;

	rc = sd_bus_message_append(reply, "is", rc, stratis_get_user_message(rc));
	if (rc < 0)
		return rc;

out:
	return sd_bus_send(NULL, reply, NULL);

}

static void
iterate_svolumes_name (gpointer key, gpointer value, gpointer user_data)
{
	sd_bus_message *reply = user_data;
	char *name = key;

	sd_bus_message_append(reply, "s", name);

}

static int list_pool_volumes(sd_bus_message *message, void *userdata,
        sd_bus_error *error) {
	svolume_table_t *svolume_list;
	spool_t *spool = (spool_t *) userdata;
	svolume_t *volume = NULL;
	sd_bus_message *reply = NULL;
	int volume_list_size = 0;
	int rc, i;

	rc = sd_bus_message_new_method_return(message, &reply);

	if (rc < 0) {
		rc = STRATIS_BAD_PARAM;
		goto out;
	}

	rc = sd_bus_message_open_container(reply, 'a', "s");
	if (rc < 0)
		goto out;

	rc = stratis_spool_get_volume_list(spool, &svolume_list);
	if (rc != STRATIS_OK) {
		fprintf(stderr, "Failed stratis_svolume_get_list()\n");
		goto out;
	}

	g_hash_table_foreach(svolume_list->table, iterate_svolumes_name, reply);

	rc = sd_bus_message_close_container(reply);
	if (rc < 0)
		return rc;

	rc = sd_bus_message_append(reply, "is", rc, stratis_get_user_message(rc));
	if (rc < 0)
		return rc;

out:
	return sd_bus_send(NULL, reply, NULL);

}

static int create_pool(sd_bus_message *m, void *userdata, sd_bus_error *error) {
	struct context *c = userdata;
	char *name = NULL;
	spool_t *spool;
	sdev_table_t *sdev_list = NULL;
	sdev_t *sdev = NULL;
	int rc;
	int raid_type = STRATIS_VOLUME_RAID_TYPE_UNKNOWN;
	char *sdev_name = NULL;
	size_t length = 0;

	rc = sd_bus_message_read(m, "s", &name);

	if (rc < 0) {
		rc = STRATIS_BAD_PARAM;
		goto out;
	}

	rc = find_spool(name, &spool);

	/*
	 * Make sure the object doesn't already exist.
	 */
	if (rc != STRATIS_NOTFOUND && rc != STRATIS_NULL) {
		rc = STRATIS_DUPLICATE_NAME;
		goto out;
	}

	rc = stratis_sdev_table_create(&sdev_list);

	if (rc != STRATIS_OK)
		goto out;

	rc = sd_bus_message_enter_container(m, 'a', "s");
	if (rc < 0)
		goto out;

	for (;;) {
		rc = sd_bus_message_read_basic(m, SD_BUS_TYPE_STRING, &sdev_name);

		if (rc < 0)
			goto out;
		if (rc == 0)
			break;

		rc = stratis_sdev_create(&sdev, spool,
					sdev_name, STRATIS_DEV_TYPE_REGULAR);

		if (rc != STRATIS_OK)
			goto out;

		rc = stratis_sdev_table_add(sdev_list, sdev);

	}
	rc = sd_bus_message_exit_container(m);
	if (rc < 0)
		goto out;

	rc = sd_bus_message_read(m, "i", &raid_type);
	if (rc < 0)
		goto out;

	// TODO free the sdev_list - have create make a copy
	rc = stratis_spool_create(&spool, name, sdev_list, raid_type);

	if (rc < 0)
		goto out;

	rc = sync_spool(spool);

	if (rc < 0)
		goto out;

out:
	return sd_bus_reply_method_return(m, "sis", spool->dbus_name, rc,
	        stratis_get_user_message(rc));
}

static int unref_sdev(sdev_t *sdev) {

	int rc = STRATIS_OK;

	if (sdev == NULL || sdev->slot == NULL)
		return STRATIS_NULL;

	sdev->slot = sd_bus_slot_unref(sdev->slot);
	rc = sd_bus_emit_object_removed(bus, sdev->dbus_name);

	return rc;
}

static int unref_scache(scache_t *scache) {

	int rc = STRATIS_OK;

	if (scache == NULL || scache->slot == NULL)
		return STRATIS_NULL;

	scache->slot = sd_bus_slot_unref(scache->slot);
	rc = sd_bus_emit_object_removed(bus, scache->dbus_name);

	return rc;
}

static int unref_volume(svolume_t *volume) {

	int rc = STRATIS_OK;

	if (volume == NULL || volume->slot == NULL)
		return STRATIS_NULL;

	volume->slot = sd_bus_slot_unref(volume->slot);
	rc = sd_bus_emit_object_removed(bus, volume->dbus_name);

	return rc;
}


static void
iterate_svolumes_unref (gpointer key, gpointer value, gpointer user_data)
{
	svolume_t *svolume = value;

	unref_volume(svolume);

}

static void
iterate_sdev_unref (gpointer key, gpointer value, gpointer user_data)
{
	sdev_t *sdev = value;

	unref_sdev(sdev);

}
static void
iterate_scache_unref (gpointer key, gpointer value, gpointer user_data)
{
	scache_t *scache = value;

	unref_scache(scache);

}

static int unref_pool(spool_t *spool) {
	int rc = STRATIS_OK;

	if (spool == NULL || spool->slot == NULL)
		return STRATIS_NULL;

	spool->slot = sd_bus_slot_unref(spool->slot);
	rc = sd_bus_emit_object_removed(bus, spool->dbus_name);
	spool->slot = NULL;

	g_hash_table_foreach(spool->svolume_table->table,
					iterate_svolumes_unref,
					NULL);

	g_hash_table_foreach(spool->sdev_table->table,
					iterate_sdev_unref,
					NULL);

	g_hash_table_foreach(spool->scache_table->table,
					iterate_scache_unref,
					NULL);

	return rc;
}

static int destroy_pool(sd_bus_message *m, void *userdata, sd_bus_error *error) {
	int rc = STRATIS_OK;
	spool_t *spool = NULL;
	char dbus_name[MAX_STRATIS_NAME_LEN] = "";
	char *name = NULL;

	rc = sd_bus_message_read(m, "s", &name);

	if (rc < 0) {
		rc = STRATIS_BAD_PARAM;
		goto out;
	}

	rc = find_spool(name, &spool);

	if (rc != STRATIS_OK)
		goto out;

	strcpy(dbus_name, spool->dbus_name);

	unref_pool(spool);

	rc = stratis_spool_destroy(spool);

	out: return sd_bus_reply_method_return(m, "sis", dbus_name, rc,
	        stratis_get_user_message(rc));

}

static int get_pool_object_path(sd_bus_message *m, void *userdata, sd_bus_error *error) {
	int rc = STRATIS_OK;
	char *name = NULL;
	spool_t *spool = NULL;
	char *dbus_name = "";

	rc = sd_bus_message_read(m, "s", &name);

	if (rc < 0) {
		rc = STRATIS_BAD_PARAM;
		goto out;
	}

	rc = stratis_spool_get(&spool, name);

	if (spool != NULL) {
		dbus_name = spool->dbus_name;
	}
out:
	return sd_bus_reply_method_return(m, "sis", dbus_name, rc,
		        stratis_get_user_message(rc));
}


static int get_volume_object_path(sd_bus_message *m, void *userdata, sd_bus_error *error) {
	int rc = STRATIS_OK;
	char *volumename = NULL, *poolname = NULL;
	svolume_t *svolume = NULL;
	char *dbus_name = "";

	rc = sd_bus_message_read(m, "ss", &poolname, &volumename);

	if (rc < 0) {
		rc = STRATIS_BAD_PARAM;
		goto out;
	}

	rc = stratis_svolume_get(&svolume, poolname, volumename);

	if (svolume != NULL) {
		dbus_name = svolume->dbus_name;
	}
out:
	return sd_bus_reply_method_return(m, "sis", dbus_name, rc,
		        stratis_get_user_message(rc));
}

static int get_dev_object_path(sd_bus_message *m, void *userdata, sd_bus_error *error) {
	int rc = STRATIS_OK;
	char *dev_name = NULL;
	sdev_t *sdev = NULL;
	char *dbus_name = "";

	rc = sd_bus_message_read(m, "s", &dev_name);

	if (rc < 0) {
		rc = STRATIS_BAD_PARAM;
		goto out;
	}


	rc = stratis_dev_get(&sdev, dev_name);

	if (sdev != NULL) {
		dbus_name = sdev->dbus_name;
	}
out:
	return sd_bus_reply_method_return(m, "sis", dbus_name, rc,
		        stratis_get_user_message(rc));
}

static int create_volumes(sd_bus_message *m, void *userdata, sd_bus_error *error) {
	sd_bus_message *reply = NULL;
	spool_t *spool = userdata;
	svolume_t *svolume;
	char *volume_name = "", *mount_point = "", *quota = "";
	char *n = "";
	int rc;
	int stratis_rc = STRATIS_OK;

	rc = sd_bus_message_enter_container(m, 'a', "(sss)");
	if (rc < 0)
		goto out;

	rc = sd_bus_message_new_method_return(m, &reply);

	if (rc < 0) {
		rc = STRATIS_BAD_PARAM;
		goto out;
	}

	rc = sd_bus_message_open_container(reply, 'a', "(sis)");

	if (rc < 0)
		goto out;

	for (;;) {

		rc = sd_bus_message_read(m, "(sss)", &volume_name, &mount_point, &quota);

		if (rc < 0)
			goto out;
		if (rc == 0)
			break;

		rc = stratis_svolume_create(&svolume, spool,
				volume_name, mount_point, quota);

		if (rc != STRATIS_OK) {
			stratis_rc = STRATIS_LIST_FAILURE;
		}

		sync_volume(svolume, spool);

		sd_bus_message_append(reply, "(sis)", svolume->dbus_name, rc, stratis_get_user_message(rc));

		if (rc < 0)
			goto out;
	}

	rc = sd_bus_message_close_container(reply);
	if (rc < 0)
		goto out;

	rc = sd_bus_message_exit_container(m);
	if (rc < 0)
		goto out;

	rc = sd_bus_message_append(reply, "is", stratis_rc, stratis_get_user_message(stratis_rc));
	if (rc < 0)
		goto out;

	return sd_bus_send(NULL, reply, NULL);

out:
	return rc;
}
static int set_mount_point_volume(sd_bus_message *m, void *userdata, sd_bus_error *error) {
	int rc = STRATIS_OK;

	return rc;
}

static int set_quota_volume(sd_bus_message *m, void *userdata, sd_bus_error *error) {
	int rc = STRATIS_OK;

	return rc;
}

static int rename_volume(sd_bus_message *m, void *userdata, sd_bus_error *error) {
	int rc = STRATIS_OK;

	return rc;
}



static int destroy_volumes(sd_bus_message *m, void *userdata,
        sd_bus_error *error) {
	spool_t *spool = userdata;
	sd_bus_message *reply = NULL;
	svolume_t *volume = NULL;
	char dbus_name[MAX_STRATIS_NAME_LEN] = "";
	char *volume_name = NULL;
	int rc;
	int stratis_rc = STRATIS_OK;
	int failure = FALSE;

	rc = sd_bus_message_enter_container(m, 'a', "s");
	if (rc < 0)
		goto out;

	rc = sd_bus_message_new_method_return(m, &reply);

	if (rc < 0) {
		rc = STRATIS_BAD_PARAM;
		goto out;
	}

	rc = sd_bus_message_open_container(reply, 'a', "(sis)");

	if (rc < 0)
		goto out;

	for (;;) {
		rc = sd_bus_message_read_basic(m, SD_BUS_TYPE_STRING, &volume_name);

		if (rc < 0)
			goto out;
		if (rc == 0)
			break;

		rc = find_volume(volume_name, spool, &volume);

		if (rc != STRATIS_OK) {
			failure = TRUE;
			stratis_rc = rc;
		} else {

			strcpy(dbus_name, volume->dbus_name);
			unref_volume(volume);


			rc = stratis_svolume_destroy(volume);

			if (rc != STRATIS_OK) {
				failure = TRUE;
				stratis_rc = rc;
			}
		}

		rc = sd_bus_message_append(reply, "(sis)", dbus_name, rc, stratis_get_user_message(rc));

		if (rc < 0)
			goto out;

	}
	if (failure == TRUE) {
		stratis_rc = STRATIS_LIST_FAILURE;
	}

	rc = sd_bus_message_close_container(reply);
	if (rc < 0)
		goto out;

	rc = sd_bus_message_exit_container(m);
	if (rc < 0)
		goto out;

	rc = sd_bus_message_append(reply, "is", stratis_rc, stratis_get_user_message(stratis_rc));
	if (rc < 0)
		goto out;

	return sd_bus_send(NULL, reply, NULL);

out:

	return rc;

}

static int add_cache_devs(sd_bus_message *m, void *userdata, sd_bus_error *error) {
	spool_t *spool = userdata;
	char *cache_name;
	sdev_t *sdev = NULL;
	sdev_table_t *scache_table;
	int rc;

	rc = stratis_sdev_table_create(&scache_table);

	if (rc != STRATIS_OK)
		goto out;

	rc = sd_bus_message_enter_container(m, 'a', "s");

	if (rc < 0)
		goto out;

	for (;;) {
		rc = sd_bus_message_read_basic(m, SD_BUS_TYPE_STRING, &cache_name);

		if (rc < 0)
			goto out;
		if (rc == 0)
			break;

		rc = stratis_sdev_create(&sdev, spool,
				cache_name, STRATIS_DEV_TYPE_REGULAR);

		if (rc != STRATIS_OK)
			goto out;
		rc = stratis_sdev_table_add(scache_table, sdev);

	}

	sd_bus_message_exit_container(m);

	rc = stratis_spool_add_cache_devs(spool, scache_table);

	if (rc != STRATIS_OK)
		goto out;

out:
	return sd_bus_reply_method_return(m, "sis", spool->dbus_name, rc,
	        stratis_get_user_message(rc));

}

static int remove_dev(sd_bus_message *m, void *userdata, sd_bus_error *error) {
	struct context *c = userdata;
	const char *s;
	char *n = "";
	int r;

	r = sd_bus_message_read(m, "s", &s);
	if (r < 0)
		goto out;

	printf("remove_dev() called, got %s, returning %s", s, n);

	return sd_bus_reply_method_return(m, "is", r, n);

out:
	return r;
}

static int remove_cache(sd_bus_message *m, void *userdata, sd_bus_error *error) {
	struct context *c = userdata;
	const char *s;
	char *n = "";
	int r;

	r = sd_bus_message_read(m, "s", &s);
	if (r < 0)
		goto out;

	printf("remove cache() called, got %s, returning %s", s, n);

	return sd_bus_reply_method_return(m, "is", r, n);

out:
	return r;
}

static void
iterate_cache (gpointer key, gpointer value, gpointer user_data)
{
	sd_bus_message *reply = user_data;
	char *name = key;

	sd_bus_message_append(reply, "s", name);

}

static int list_cache_devs(sd_bus_message *message, void *userdata,
        sd_bus_error *error) {
	scache_table_t *cache_list;
	spool_t *spool = (spool_t *) userdata;
	svolume_t *volume = NULL;
	sd_bus_message *reply = NULL;
	int cache_list_size = 0;
	int rc, i;

	rc = sd_bus_message_new_method_return(message, &reply);

	if (rc < 0) {
		rc = STRATIS_BAD_PARAM;
		goto out;
	}

	rc = sd_bus_message_open_container(reply, 'a', "s");
	if (rc < 0)
		goto out;

	rc = stratis_spool_get_cache_dev_table(spool, &cache_list);
	if (rc != STRATIS_OK) {
		fprintf(stderr, "Failed stratis_spool_get_cache_dev_table()\n");
		goto out;
	}

	g_hash_table_foreach(cache_list->table, iterate_cache, reply);

	rc = sd_bus_message_close_container(reply);
	if (rc < 0)
		return rc;

	rc = sd_bus_message_append(reply, "is", rc, stratis_get_user_message(rc));
	if (rc < 0)
		return rc;

out:
	return sd_bus_send(NULL, reply, NULL);

}
static int get_handler(sd_bus *bus, const char *path, const char *interface, const char *property, sd_bus_message *reply, void *userdata, sd_bus_error *error) {
		svolume_t *c = userdata;
        char *return_str;
        int rc;

    	if (rc < 0)
    		goto out;

    	if (strcmp(property, VOLUME_MOUNT_POINT ) == 0) {
    		return_str = c->mount_point;
    		goto out;
    	}

    	if (strcmp(property, VOLUME_QUOTA ) == 0) {
    		return_str = c->quota;
    		goto out;
    	}

    	if (strcmp(property, VOLUME_NAME ) == 0) {
    		return_str = c->name;
    		goto out;
    	}
out:
		rc = sd_bus_message_append(reply, "s", return_str);
        return 1;
}


static int set_handler(sd_bus *bus, const char *path, const char *interface,
		const char *property, sd_bus_message *value, void *userdata,
		sd_bus_error *error) {
        svolume_t *c = userdata;
        const char *s;
        char *n;
        int rc = 0;


    	if (strcmp(property, VOLUME_MOUNT_POINT ) == 0) {
    		strcpy(c->mount_point, property);
    		return 0;
    	}

    	if (strcmp(property, VOLUME_QUOTA ) == 0) {
    		strcpy(c->quota, property);
    		return 0;
    	}

    	if (strcmp(property, VOLUME_NAME ) == 0) {
    		strcpy(c->name, property);
    		return 0;
    	}

out:
		return rc;
}


static const sd_bus_vtable stratis_manager_vtable[] = {
	SD_BUS_VTABLE_START(0),
	SD_BUS_PROPERTY("Version", "s", property_get_version, 0, SD_BUS_VTABLE_PROPERTY_CONST),
	SD_BUS_PROPERTY("LogLevel", "s", property_get_log_level,  0, SD_BUS_VTABLE_PROPERTY_CONST),
	SD_BUS_METHOD("ListPools", NULL, "asis", list_pools, SD_BUS_VTABLE_UNPRIVILEGED),
	SD_BUS_METHOD("CreatePool", "sasi", "sis", create_pool, SD_BUS_VTABLE_UNPRIVILEGED),
	SD_BUS_METHOD("DestroyPool", "s", "sis", destroy_pool, SD_BUS_VTABLE_UNPRIVILEGED),
	SD_BUS_METHOD("GetPoolObjectPath", "s", "sis", get_pool_object_path, SD_BUS_VTABLE_UNPRIVILEGED),
	SD_BUS_METHOD("GetVolumeObjectPath", "ss", "sis", get_volume_object_path, SD_BUS_VTABLE_UNPRIVILEGED),
	SD_BUS_METHOD("GetDevObjectPath", "s", "sis", get_dev_object_path, SD_BUS_VTABLE_UNPRIVILEGED),
	SD_BUS_METHOD("GetErrorCodes", NULL, "a(is)", get_error_codes, SD_BUS_VTABLE_UNPRIVILEGED),

	SD_BUS_VTABLE_END
};

static const sd_bus_vtable spool_vtable[] = {
	SD_BUS_VTABLE_START(0),
	SD_BUS_PROPERTY(POOL_NAME, "s", get_spool_property, 0,
			SD_BUS_VTABLE_PROPERTY_CONST),
	SD_BUS_PROPERTY(POOL_ID, "i", get_spool_property, 0,
			SD_BUS_VTABLE_PROPERTY_CONST),
	SD_BUS_WRITABLE_PROPERTY("Size", "u", NULL, NULL,
			offsetof(spool_t, size), 0),
	SD_BUS_METHOD("CreateVolumes", "a(sss)", "a(sis)is", create_volumes, SD_BUS_VTABLE_UNPRIVILEGED),
	SD_BUS_METHOD("SetMountPoint", "s", "is", set_mount_point_volume, SD_BUS_VTABLE_UNPRIVILEGED),
	SD_BUS_METHOD("SetQuota", "s", "is", set_quota_volume, SD_BUS_VTABLE_UNPRIVILEGED),
	SD_BUS_METHOD("DestroyVolumes", "as", "a(sis)is", destroy_volumes, SD_BUS_VTABLE_UNPRIVILEGED),
	SD_BUS_METHOD("ListVolumes", NULL, "asis", list_pool_volumes, SD_BUS_VTABLE_UNPRIVILEGED),
	SD_BUS_METHOD("ListDevs", NULL, "asis", list_pool_devs, SD_BUS_VTABLE_UNPRIVILEGED),
	SD_BUS_METHOD("ListCacheDevs", NULL, "asis", list_cache_devs, SD_BUS_VTABLE_UNPRIVILEGED),
	SD_BUS_METHOD("AddCacheDevs", "as", "sis", add_cache_devs, SD_BUS_VTABLE_UNPRIVILEGED),
	SD_BUS_METHOD("RemoveCacheDevs", "as", "is", remove_cache, SD_BUS_VTABLE_UNPRIVILEGED),
	SD_BUS_METHOD("RemoveDevs", "as", "is", remove_dev, SD_BUS_VTABLE_UNPRIVILEGED),
//	SD_BUS_METHOD("DeviceLossImpact", "ss", "is", remove_cache, SD_BUS_VTABLE_UNPRIVILEGED),
	SD_BUS_VTABLE_END
};

static const sd_bus_vtable svolume_vtable[] = {
	SD_BUS_VTABLE_START(0),
	SD_BUS_PROPERTY(VOLUME_NAME, "s", get_svolume_property, 0,
				SD_BUS_VTABLE_PROPERTY_CONST),
	SD_BUS_PROPERTY(VOLUME_ID, "s", get_svolume_property, 0,
				SD_BUS_VTABLE_PROPERTY_CONST),
	SD_BUS_METHOD("Rename", "s", "is", rename_volume, 0),
	SD_BUS_WRITABLE_PROPERTY(VOLUME_QUOTA, "s", get_handler, set_handler,
				0, 0),
	SD_BUS_WRITABLE_PROPERTY(VOLUME_MOUNT_POINT, "s", get_handler, set_handler,
				0, 0),
	SD_BUS_VTABLE_END
};

static const sd_bus_vtable sdev_vtable[] = {
	SD_BUS_VTABLE_START(0),
	SD_BUS_PROPERTY(DEV_NAME, "s", get_sdev_property, 0,
				SD_BUS_VTABLE_PROPERTY_CONST),
	SD_BUS_PROPERTY(DEV_ID, "i", get_sdev_property, 0,
				SD_BUS_VTABLE_PROPERTY_CONST),
	SD_BUS_WRITABLE_PROPERTY(DEV_SIZE, "u", NULL, NULL,
				offsetof(sdev_t, size), 0),
	SD_BUS_WRITABLE_PROPERTY(DEV_TYPE, "u", NULL, NULL,
				offsetof(sdev_t, type), 0),
	SD_BUS_VTABLE_END
};

static const sd_bus_vtable scache_vtable[] = {
	SD_BUS_VTABLE_START(0),
	SD_BUS_PROPERTY(CACHE_NAME, "s", get_sdev_property, 0,
				SD_BUS_VTABLE_PROPERTY_CONST),
	SD_BUS_PROPERTY(CACHE_ID, "i", get_sdev_property, 0,
				SD_BUS_VTABLE_PROPERTY_CONST),
	SD_BUS_VTABLE_END
};

static int sync_volume(svolume_t *volume, spool_t *spool) {
	int rc = STRATIS_OK;

	snprintf(volume->dbus_name, 256, "%s/%d", STRATIS_BASE_PATH,
		        stratis_svolume_get_id(volume));

	rc = sd_bus_add_object_vtable(bus, &(volume->slot), volume->dbus_name,
			STRATIS_VOLUME_BASE_INTERFACE, svolume_vtable, volume);

	if (rc < 0) {
		fprintf(stderr, "Failed sd_bus_add_object_vtable for svolume : %s\n",
		        strerror(rc));
		goto out;
	}

out:
	return rc;
}

static void
iterate_sdevs_vtable (gpointer key, gpointer value, gpointer user_data)
{
	sdev_t *sdev = value;
	char *sdev_name = key;

	snprintf(sdev->dbus_name, 256, "%s/%d", STRATIS_BASE_PATH,
	        stratis_sdev_get_id(sdev));

	sd_bus_add_object_vtable(bus, &(sdev->slot), sdev->dbus_name,
			STRATIS_DEV_BASE_INTERFACE, sdev_vtable, sdev);

}


static int sync_spool(spool_t *spool) {
	int rc = STRATIS_OK;
	int table_size;
	char spool_name[256];
	spool_table_t *spool_list;
	sdev_table_t *sdev_table;

	snprintf(spool->dbus_name, 256, "%s/%d", STRATIS_BASE_PATH,
	        stratis_spool_get_id(spool));

	rc = sd_bus_add_object_vtable(bus, &(spool->slot), spool->dbus_name,
				STRATIS_POOL_BASE_INTERFACE, spool_vtable, spool);

	if (rc < 0) {
		fprintf(stderr, "Failed sd_bus_add_object_vtable for spool: %s\n",
		        strerror(rc));
		goto out;
	}

	rc = stratis_spool_get_dev_table(spool, &sdev_table);

	if (rc != STRATIS_OK) {
		fprintf(stderr, "Failed stratis_svolume_get_list()\n");
		goto out;
	}
	stratis_sdev_table_size(sdev_table, &table_size);
	g_hash_table_foreach(sdev_table->table, iterate_sdevs_vtable, sdev_table);

out:
	return rc;
}

void * stratis_main_loop(void * ap) {
	int r;
	sd_bus_slot *slot = NULL;

	r = sd_bus_open_user(&bus);

	if (r < 0) {
		fprintf(stderr, "Failed to connect to system bus: %s\n", strerror(-r));
		goto finish;
	}

	r = sd_bus_add_object_vtable(bus, &slot, STRATIS_BASE_PATH,
	STRATIS_MANAGER_INTERFACE, stratis_manager_vtable, NULL);

	if (r < 0) {
		fprintf(stderr,
		        "Failed sd_bus_add_object_vtable for stratis_manager_vtable: %s\n",
		        strerror(-r));
		goto finish;
	}

	r = sd_bus_add_object_manager(bus, &slot, STRATIS_BASE_PATH);

	if (r < 0) {
		fprintf(stderr, "Failed to sd_bus_add_object_manager: %s\n",
		        strerror(-r));
		goto finish;
	}

	/* Take a well-known service name so that clients can find us */
	r = sd_bus_request_name(bus, STRATIS_BASE_SERVICE, 0);

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
		r = sd_bus_wait(bus, (uint64_t) -1);
		if (r < 0) {
			fprintf(stderr, "Failed to wait on bus: %s\n", strerror(-r));
			goto finish;
		}
	}

	finish: sd_bus_slot_unref(slot);
	sd_bus_flush_close_unref(bus);

	return NULL;
}
