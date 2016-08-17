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
#include <string.h>
#include "libstratis.h"
#include "stratis-common.h"

spool_table_t *the_spool_table = NULL;

static int dbus_id = 0;

/*
 * Stratis
 */

void stratis_initialize() {

	if (the_spool_table == NULL) {
		the_spool_table = malloc(sizeof(spool_table_t));
		the_spool_table->table = g_hash_table_new (g_str_hash, g_str_equal);
	}

}
int stratis_cache_get(scache_t **scache, char *name) {
	GList *values;
	int list_size, i;
	int rc = STRATIS_OK;
	spool_t *spool;

	if (scache == NULL) {
		rc = STRATIS_MALLOC;
		goto out;
	}

	if (the_spool_table->table == NULL) {
		rc = STRATIS_DEV_NOTFOUND;
		goto out;
	}

	values = g_hash_table_get_values(the_spool_table->table);
	list_size = g_list_length(values);

	for (i = 0; i < list_size; i++) {
		spool = g_list_nth_data(values, i);

		*scache = g_hash_table_lookup(spool->scache_table->table, name);

		if (*scache != NULL)
			break;
	}

	g_list_free(values);

	if (*scache == NULL)
		rc = STRATIS_CACHE_NOTFOUND;
out:
	return rc;
}

int stratis_sdev_get(sdev_t **sdev, char *name) {
	GList *values;
	int list_size, i;
	int rc = STRATIS_OK;
	spool_t *spool;

	if (sdev == NULL) {
		rc = STRATIS_MALLOC;
		goto out;
	}

	if (the_spool_table->table == NULL) {
		rc = STRATIS_DEV_NOTFOUND;
		goto out;
	}

	values = g_hash_table_get_values(the_spool_table->table);
	list_size = g_list_length(values);

	for (i = 0; i < list_size; i++) {
		spool = g_list_nth_data(values, i);

		*sdev = g_hash_table_lookup(spool->sdev_table->table, name);

		if (*sdev != NULL)
			break;
	}

	g_list_free(values);

	if (*sdev == NULL)
		rc = STRATIS_DEV_NOTFOUND;
out:
	return rc;
}
/*
 * Pools
 */

int stratis_spool_create(spool_t **spool, const char *name,
        sdev_table_t *disk_list, int raid_level) {
	int rc = STRATIS_OK;
	spool_t *return_spool = NULL;

	return_spool = malloc(sizeof(spool_t));

	if (return_spool == NULL) {
		rc = STRATIS_MALLOC;
		goto out;
	}

	return_spool->svolume_table = malloc(sizeof(svolume_table_t));

	if (return_spool->svolume_table == NULL) {
		rc = STRATIS_MALLOC;
		goto out;
	}

	return_spool->sdev_table = malloc(sizeof(sdev_table_t));

	if (return_spool->sdev_table == NULL) {
		rc = STRATIS_MALLOC;
		goto out;
	}

	return_spool->scache_table = malloc(sizeof(sdev_table_t));

	if (return_spool->scache_table == NULL) {
		rc = STRATIS_MALLOC;
		goto out;
	}

	return_spool->slot = NULL;
	return_spool->svolume_table->table = g_hash_table_new (g_str_hash, g_str_equal);
	return_spool->scache_table->table = g_hash_table_new (g_str_hash, g_str_equal);
	return_spool->sdev_table->table = g_hash_table_new (g_str_hash, g_str_equal);
	return_spool->id = dbus_id++;
	return_spool->size = 32767;
	strncpy(return_spool->name, name, MAX_STRATIS_NAME_LEN);

	/* TODO should we duplicate the disk_list? */
	return_spool->sdev_table = disk_list;

	g_hash_table_insert(the_spool_table->table, return_spool->name, return_spool);

	*spool = return_spool;
	return rc;

	out:

	if (return_spool != NULL) {

		if (return_spool->svolume_table != NULL) {
			// TODO fix memory leak of list elements
			free(return_spool->svolume_table);
		}
		if (return_spool->sdev_table != NULL) {
			// TODO fix memory leak of list elements
			free(return_spool->sdev_table);
		}
		free(return_spool);
	}

	return rc;

}

int stratis_spool_destroy(spool_t *spool) {
	int rc = STRATIS_OK;

	if (spool == NULL) {
		rc = STRATIS_NULL;
		goto out;
	}
	gboolean found = g_hash_table_remove(the_spool_table->table, spool->name);

	if (found == FALSE) {
		rc = STRATIS_NOTFOUND;
		goto out;
	}

	if (spool->svolume_table != NULL) {
		g_hash_table_remove_all(spool->svolume_table->table);
	}

	if (spool->scache_table != NULL) {
		g_hash_table_remove_all(spool->scache_table->table);
	}

	if (spool->sdev_table != NULL) {
		g_hash_table_remove_all(spool->sdev_table->table);
	}

	free(spool);
out:
	return rc;
}

int stratis_spool_get(spool_t **spool, char *name) {
	int rc = STRATIS_OK;

	if (spool == NULL || the_spool_table == NULL) {
		return STRATIS_NULL;
	}

	*spool = g_hash_table_lookup(the_spool_table->table, name);

	if (*spool == NULL)
		rc = STRATIS_NOTFOUND;

	return rc;
}

char *stratis_spool_get_name(spool_t *spool) {

	if (spool == NULL) {
		return NULL;
	}

	return spool->name;
}


int stratis_spool_get_id(spool_t *spool) {

	if (spool == NULL) {
		return -1;
	}

	return spool->id;
}

int stratis_spool_get_list(spool_table_t **spool_list) {

	if (spool_list == NULL || *spool_list == NULL)
		return STRATIS_NULL;

	*spool_list = the_spool_table;

	return STRATIS_OK;
}

int stratis_spool_get_volume_list(spool_t *spool, svolume_table_t **svolume_list) {

	int rc = STRATIS_OK;

	if (spool == NULL || svolume_list == NULL)
		return STRATIS_NULL;

	*svolume_list = spool->svolume_table;

	return rc;
}

int stratis_spool_get_dev_table(spool_t *spool, sdev_table_t **sdev_table) {

	int rc = STRATIS_OK;

	if (spool == NULL || sdev_table == NULL)
		return STRATIS_NULL;

	*sdev_table = spool->sdev_table;

	return rc;
}

int stratis_spool_add_volume(spool_t *spool, svolume_t *volume) {
	int rc = STRATIS_OK;

	if (spool == NULL || spool->svolume_table == NULL|| volume == NULL)
		return STRATIS_NULL;

	g_hash_table_insert(spool->svolume_table->table, volume->name, volume);

	return rc;
}


static void
iterate_dev_add (gpointer key, gpointer value, gpointer user_data)
{
	GHashTable *table = user_data;

	g_hash_table_insert(table, key, value);

}

int stratis_spool_add_cache_devs(spool_t *spool, sdev_table_t *sdev_table) {
	int rc = STRATIS_OK;

	if (spool == NULL || sdev_table == NULL
				|| spool->scache_table == NULL || sdev_table == NULL)
		return STRATIS_NULL;

	g_hash_table_foreach(sdev_table->table, iterate_dev_add,
			spool->sdev_table->table);

	return rc;
}

int stratis_spool_add_devs(spool_t *spool, sdev_table_t *sdev_table) {
	int rc = STRATIS_OK;

	if (spool == NULL || sdev_table == NULL
				|| spool->scache_table == NULL || sdev_table == NULL)
		return STRATIS_NULL;

	g_hash_table_foreach(sdev_table->table, iterate_dev_add,
			spool->sdev_table->table);

	return rc;
}

static void
iterate_dev_remove (gpointer key, gpointer value, gpointer user_data)
{
	GHashTable *table = user_data;

	g_hash_table_remove(table, key);

}

int stratis_spool_remove_cache_devs(spool_t *spool, sdev_table_t *scache_table) {

	g_hash_table_foreach(scache_table->table, iterate_dev_remove,
			spool->sdev_table->table);

	return STRATIS_OK;
}

int stratis_spool_remove_devs(spool_t *spool, sdev_table_t *sdev_table) {

	g_hash_table_foreach(sdev_table->table, iterate_dev_remove,
			spool->sdev_table->table);

	return STRATIS_OK;
}

int stratis_spool_remove_dev(spool_t *spool, char *name) {

	int removed, rc;

	removed = g_hash_table_remove(spool->sdev_table->table, name);

	if (removed == TRUE) {
		rc = STRATIS_OK;
	} else {
		rc = STRATIS_NOTFOUND;
	}
	return rc;
}

int stratis_spool_get_cache_dev_table(spool_t *spool, scache_table_t **scache_table) {
	if (spool == NULL || spool->sdev_table == NULL)
		return STRATIS_NULL;

	// TODO make copy
	*scache_table = spool->scache_table;

	return STRATIS_OK;
}


int stratis_spool_table_find(spool_table_t *spool_table, spool_t **spool,
        char *name) {
	GHashTable *l;
	if (spool == NULL || spool_table == NULL)
		return STRATIS_NULL;

	*spool = g_hash_table_lookup(spool_table->table, name);

	if (*spool == NULL)
		return STRATIS_NOTFOUND;

	return STRATIS_OK;

}

int stratis_spool_list_size(spool_table_t *spool_list, int *list_size) {
	int rc = STRATIS_OK;

	if (spool_list == NULL || list_size == NULL)
		return STRATIS_NULL;

	if (spool_list->table == NULL)
		*list_size = 0;
	else
		*list_size = g_hash_table_size(spool_list->table);

	return rc;
}

/*
 * Volumes
 */
int stratis_svolume_create(svolume_t **svolume, spool_t *spool, char *name,
        	char *mount_point, char *quota) {
	int rc = STRATIS_OK;

	svolume_t *return_volume;

	return_volume = malloc(sizeof(svolume_t));

	if (return_volume == NULL)
		return STRATIS_MALLOC;

	strncpy(return_volume->name, name, MAX_STRATIS_NAME_LEN);
	strncpy(return_volume->mount_point, mount_point, MAX_STRATIS_NAME_LEN);
	strncpy(return_volume->quota, quota, MAX_STRATIS_NAME_LEN);
	return_volume->id = dbus_id++;
	return_volume->parent_spool = spool;
	rc = stratis_spool_add_volume(spool, return_volume);

	if (rc != STRATIS_OK)
		goto out;

	*svolume = return_volume;

	out: return rc;
}

int stratis_svolume_destroy(svolume_t *svolume) {
	int rc = STRATIS_OK;

	return rc;
}

int stratis_svolume_get(svolume_t **svolume, char *poolname, char *volumename) {
	int rc = STRATIS_OK;
	spool_t *spool = NULL;

	if (svolume == NULL || the_spool_table == NULL ||
			poolname == NULL || volumename == NULL  ) {
		return STRATIS_NULL;
	}

	spool = g_hash_table_lookup(the_spool_table->table, poolname);

	if (spool == NULL || spool->svolume_table == NULL)
			rc = STRATIS_NOTFOUND;

	*svolume = g_hash_table_lookup(spool->svolume_table->table, volumename);

	return rc;
}

char *stratis_svolume_get_name(svolume_t *svolume) {

	if (svolume == NULL) {
		return NULL;
	}

	return svolume->name;
}

int stratis_svolume_get_id(svolume_t *svolume) {

	if (svolume == NULL) {
		return -1;
	}

	return svolume->id;
}

char *stratis_svolume_get_mount_point(svolume_t *svolume) {
	if (svolume == NULL) {
		return NULL;
	}

	return svolume->mount_point;
}

int svolume_compare(gconstpointer a, gconstpointer b) {
	char *name = (char *) b;
	svolume_t *svolume = (svolume_t *) a;

	return strcmp(name, svolume->name);
}


int stratis_svolume_table_find(svolume_table_t *svolume_table, svolume_t **svolume,
	        char *name)
{
	GHashTable *l;
	if (svolume == NULL || svolume_table == NULL)
		return STRATIS_NULL;

	*svolume = g_hash_table_lookup(svolume_table->table, name);

	if (*svolume == NULL)
		return STRATIS_NOTFOUND;

	return STRATIS_OK;

}


int stratis_svolume_table_create(svolume_table_t *svolume_table) {
	int rc = STRATIS_OK;

	return rc;
}

int stratis_svolume_table_destroy(svolume_table_t *svolume_table) {
	int rc = STRATIS_OK;

	return rc;
}

int stratis_svolume_table_size(svolume_table_t *svolume_table, int *list_size) {
	int rc = STRATIS_OK;

	if (svolume_table == NULL || list_size == NULL)
		return STRATIS_NULL;

	if (svolume_table->table == NULL)
		*list_size = 0;
	else
		*list_size = g_hash_table_size(svolume_table->table);

	return rc;
}


int stratis_svolume_table_eligible_disks(sdev_table_t **disk_table) {
	int rc = STRATIS_OK;

	return rc;
}
int stratis_svolume_table_devs(spool_t *spool, sdev_table_t **disk_table) {
	int rc = STRATIS_OK;

	return rc;
}


/*
 * Devs
 */
int stratis_sdev_create(sdev_t **sdev, spool_t *spool,
			char *name, int type) {
	int rc = STRATIS_OK;

	sdev_t *return_sdev;

	return_sdev = malloc(sizeof(sdev_t));
	if (return_sdev == NULL)
		return STRATIS_MALLOC;

	strncpy(return_sdev->name, name, MAX_STRATIS_NAME_LEN);

	return_sdev->id = dbus_id++;
	return_sdev->parent_spool = spool;

	*sdev = return_sdev;
	return rc;
}

char *stratis_sdev_get_name(sdev_t *sdev) {

	if (sdev == NULL) {
		return NULL;
	}

	return sdev->name;
}

int stratis_sdev_get_id(sdev_t *sdev) {

	if (sdev == NULL) {
		return -1;
	}

	return sdev->id;
}

/*
 * Device Lists
 */
int stratis_sdev_table_create(sdev_table_t **sdev_table) {
	int rc = STRATIS_OK;
	sdev_table_t *return_sdev_list;

	return_sdev_list = malloc(sizeof(sdev_table_t));
	if (return_sdev_list == NULL)
		return STRATIS_MALLOC;

	return_sdev_list->table = g_hash_table_new (g_str_hash, g_str_equal);

	*sdev_table = return_sdev_list;
	return rc;
}

int stratis_sdev_table_destroy(sdev_table_t *sdev_table) {
	int rc = STRATIS_OK;

	return rc;
}

int stratis_sdev_table_add(sdev_table_t *sdev_table, sdev_t *sdev) {
	int rc = STRATIS_OK;
	char *list_copy = NULL;

	if (sdev_table == NULL ||  sdev == NULL)
		return STRATIS_NULL;

	g_hash_table_insert(sdev_table->table, sdev->name, sdev);

	return rc;
}

int stratis_sdev_table_remove(sdev_table_t **sdev_table, char *sdev) {
	int rc = STRATIS_OK;

	return rc;
}

int stratis_sdev_table_size(sdev_table_t *sdev_table, int *table_size) {
	int rc = STRATIS_OK;

	if (sdev_table == NULL || table_size == NULL)
		return STRATIS_NULL;

	if (sdev_table->table == NULL)
		*table_size = 0;
	else
		*table_size = g_hash_table_size(sdev_table->table);


	return rc;
}

