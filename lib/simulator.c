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

spool_list_t *the_spool_list = NULL;

static int pool_id 		= 0;
static int volume_id 	= 0;

/*
 * Pools
 */

int stratis_spool_create(spool_t **spool,
		const char *name,
		sdev_list_t *disk_list,
		stratis_volume_raid_type raid_level) {
	int rc = STRATIS_OK;
	spool_t *return_spool = NULL;

	return_spool = malloc(sizeof(spool_t));

    if (return_spool == NULL) {
    	rc = STRATIS_MALLOC;
    	goto out;
    }

    return_spool->svolume_list = malloc(sizeof(svolume_list_t));

    if (return_spool->svolume_list == NULL) {
    	rc = STRATIS_MALLOC;
    	goto out;
    }

	return_spool->sdev_list  = malloc(sizeof(sdev_list_t));

    if (return_spool->sdev_list == NULL) {
    	rc = STRATIS_MALLOC;
    	goto out;
    }

    return_spool->slot = NULL;
    return_spool->svolume_list->list = NULL;
	return_spool->sdev_list->list = NULL;
	return_spool->id = pool_id++;
	return_spool->size = 32767;
    strncpy(return_spool->name, name, MAX_STRATIS_NAME_LEN);

    /* TODO should we duplicate the disk_list? */
    return_spool->sdev_list = disk_list;

    if (the_spool_list == NULL) {
    	the_spool_list = malloc(sizeof(spool_list_t));
    	the_spool_list->list = NULL;
    }

    the_spool_list->list =  g_list_append (the_spool_list->list, return_spool);

	*spool = return_spool;
	return rc;

out:

	if (return_spool != NULL) {

	    if (return_spool->svolume_list != NULL) {
	    	// TODO fix memory leak of list elements
	    	free(return_spool->svolume_list);
	    }
		if (return_spool->sdev_list != NULL) {
			// TODO fix memory leak of list elements
			free(return_spool->sdev_list);
		}
		free(return_spool);
	}

	return rc;

}

int stratis_spool_destroy(spool_t *spool) {
	int rc = STRATIS_OK;

	the_spool_list->list = g_list_remove(the_spool_list->list, spool);

	if (spool->svolume_list != NULL) {
		// TODO destroy volume list
	}

	if (spool->sdev_list != NULL) {
		// TODO destroy dev list
	}

	free(spool);

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

int stratis_spool_get_list(spool_list_t **spool_list) {

	if (spool_list == NULL || *spool_list == NULL)
	    	return STRATIS_NULL;

	*spool_list = the_spool_list;

	return STRATIS_OK;
}

int stratis_spool_get_volume_list(spool_t *spool,
				svolume_list_t **svolume_list) {

	int rc = STRATIS_OK;

    if (spool == NULL || svolume_list == NULL)
    	return STRATIS_NULL;

    *svolume_list = spool->svolume_list;

	return rc;
}

int stratis_spool_get_dev_list(spool_t *spool,
				sdev_list_t **sdev_list) {

	int rc = STRATIS_OK;

    if (spool == NULL || sdev_list == NULL)
    	return STRATIS_NULL;

    *sdev_list = spool->sdev_list;

	return rc;
}

int stratis_spool_add_volume(spool_t *spool, svolume_t *volume) {
	int rc = STRATIS_OK;

    if (spool == NULL || volume == NULL)
    	return STRATIS_NULL;

	spool->svolume_list->list =  g_list_append (spool->svolume_list->list, volume);

	return rc;
}

int stratis_spool_add_dev(spool_t *spool, char *sdev) {
	int rc = STRATIS_OK;

    if (spool == NULL || sdev == NULL)
    	return STRATIS_NULL;

	spool->sdev_list->list =  g_list_append (spool->sdev_list->list, sdev);

	return rc;
}

int stratis_spool_remove_dev(spool_t *spool,  char *sdev) {
	int rc = STRATIS_OK;

	return rc;
}

int stratis_spool_list_nth(spool_list_t *spool_list,
				spool_t **spool,
				int element) {

	int rc = STRATIS_OK;

    if (spool_list == NULL || element < 0)
    	return STRATIS_NULL;

    *spool = g_list_nth_data(spool_list->list, element);

    return rc;
}

int spool_compare (gconstpointer a, gconstpointer b)
{
	char *name = (char *)b;
	spool_t *spool = (spool_t *)a;

    return strcmp (name, spool->name);
}

int stratis_spool_list_find(spool_list_t *spool_list,
				spool_t **spool,
				char *name) {
	GList *l;
	if (spool == NULL || spool_list == NULL)
		return STRATIS_NULL;

	l =  g_list_find_custom(spool_list->list, name, spool_compare);

	*spool = g_list_nth_data (l, 0);

	if (*spool == NULL)
		return STRATIS_NOTFOUND;

	return STRATIS_OK;

}

int stratis_spool_list_size(spool_list_t *spool_list, int *list_size) {
	int rc = STRATIS_OK;

    if (spool_list == NULL || list_size == NULL)
    	return STRATIS_NULL;

	if (spool_list->list == NULL)
		*list_size = 0;
	else
		*list_size = g_list_length(spool_list->list);

	return rc;
}

/*
 * Volumes
 */
int stratis_svolume_create(svolume_t **svolume,
		spool_t *spool,
		char *name,
		char *mount_point) {
	int rc = STRATIS_OK;

	svolume_t *return_volume;

	return_volume = malloc(sizeof(svolume_t));

    if (return_volume == NULL)
    	return STRATIS_MALLOC;

    strncpy(return_volume->name, name, MAX_STRATIS_NAME_LEN);
    strncpy(return_volume->mount_point, mount_point, MAX_STRATIS_NAME_LEN);
    return_volume->id = volume_id++;

    rc = stratis_spool_add_volume(spool, return_volume);

    if (rc != STRATIS_OK)
    	goto out;

    *svolume = return_volume;

out:
	return rc;
}
int stratis_svolume_destroy(svolume_t *svolume) {
	int rc = STRATIS_OK;

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

int stratis_svolume_list_create(svolume_list_t **svolume_list) {
	int rc = STRATIS_OK;

	return rc;
}

int stratis_svolume_list_destroy(svolume_list_t *svolume_list) {
	int rc = STRATIS_OK;

	return rc;
}

int stratis_svolume_list_size(svolume_list_t *svolume_list, int *list_size) {
	int rc = STRATIS_OK;

    if (svolume_list == NULL || list_size == NULL)
    	return STRATIS_NULL;

	if (svolume_list->list == NULL)
		*list_size = 0;
	else
		*list_size = g_list_length(svolume_list->list);

	return rc;
}

int stratis_svolume_list_nth(svolume_list_t *svolume_list,
				svolume_t **svolume,
				int element) {

	int rc = STRATIS_OK;

    if (svolume_list == NULL || element < 0)
    	return STRATIS_NULL;

    *svolume = g_list_nth_data(svolume_list->list, element);

    return rc;
}

int stratis_svolume_list_eligible_disks(sdev_list_t **disk_list) {
	int rc = STRATIS_OK;

	return rc;
}
int stratis_svolume_list_devs(spool_t *spool, sdev_list_t **disk_list) {
	int rc = STRATIS_OK;

	return rc;
}

/*
 * Device Lists
 */
int stratis_sdev_list_create(sdev_list_t **sdev_list) {
	int rc = STRATIS_OK;
	sdev_list_t *return_sdev_list;

	return_sdev_list = malloc(sizeof(sdev_list_t));
    if (return_sdev_list == NULL)
    	return STRATIS_MALLOC;

	return_sdev_list->list = NULL;

	*sdev_list = return_sdev_list;
	return rc;
}

int stratis_sdev_list_destroy(sdev_list_t *sdev_list) {
	int rc = STRATIS_OK;

	return rc;
}

int stratis_sdev_list_add(sdev_list_t **sdev_list, char *sdev) {
	int rc = STRATIS_OK;

	if (sdev_list == NULL || *sdev_list == NULL || sdev == NULL)
		return STRATIS_NULL;

	(*sdev_list)->list =  g_list_append ((*sdev_list)->list, sdev);

	return rc;
}

int stratis_sdev_list_remove(sdev_list_t **sdev_list, char *sdev) {
	int rc = STRATIS_OK;

	return rc;
}

int stratis_sdev_list_size(sdev_list_t *sdev_list, int *list_size) {
	int rc = STRATIS_OK;

    if (sdev_list == NULL || list_size == NULL)
    	return STRATIS_NULL;

	if (sdev_list->list == NULL)
		*list_size = 0;
	else
		*list_size = g_list_length(sdev_list->list);

	return rc;
}

int stratis_sdev_list_nth(sdev_list_t *sdev_list,
				char **sdev,
				int element) {

	int rc = STRATIS_OK;

    if (sdev_list == NULL || element < 0)
    	return STRATIS_NULL;

    *sdev = g_list_nth_data(sdev_list->list, element);

    return rc;
}
