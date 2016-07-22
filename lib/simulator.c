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

#define TEST_DEV_COUNT 20
#define TEST_POOL_COUNT 10
#define TEST_VOLUME_COUNT 5

GList * stratis_pools 	= NULL;
GList * stratis_volumes = NULL;
GList * stratis_devs 	= NULL;

static int pool_id 		= 0;
static int volume_id 	= 0;
static int dev_id 		= 0;
/*
 * Pools
 */

int stratis_spool_create(spool_t **spool,
		char *name,
		sdev_list_t *disk_list,
		stratis_volume_raid_type raid_level) {
	int rc = STRATIS_OK;
	spool_t *return_spool;

	return_spool = malloc(sizeof(spool_t));

    if (return_spool == NULL)
    	return STRATIS_MALLOC;

	return_spool->svolume_list = malloc(sizeof(svolume_list_t));

    if (return_spool->svolume_list == NULL)
    	return STRATIS_MALLOC;

    return_spool->svolume_list->list = NULL;

	return_spool->sdev_list = malloc(sizeof(sdev_list_t));

    if (return_spool->sdev_list == NULL)
    	return STRATIS_MALLOC;

    return_spool->sdev_list->list = NULL;

    strncpy(return_spool->name, name, MAX_STRATIS_NAME_LEN);

    /* TODO should we duplicate the disk_list? */
    return_spool->sdev_list = disk_list;

	stratis_pools =  g_list_append (stratis_pools, return_spool);

	*spool = return_spool;
	return rc;
}

int stratis_spool_destroy(spool_t *spool) {
	int rc = STRATIS_OK;

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
	int rc = STRATIS_OK;

	spool_list_t *return_spool_list;

	return_spool_list = malloc(sizeof(spool_list_t));

    if (return_spool_list == NULL)
    	return STRATIS_MALLOC;

	// TODO fix - don't return pointer to main list
	return_spool_list->list = stratis_pools;

	*spool_list = return_spool_list;

	return rc;
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

int stratis_spool_add_dev(spool_t *spool, sdev_t *sdev) {
	int rc = STRATIS_OK;

    if (spool == NULL || sdev == NULL)
    	return STRATIS_NULL;

	spool->sdev_list->list =  g_list_append (spool->sdev_list->list, sdev);

	return rc;
}

int stratis_spool_remove_dev(spool_t *spool,  sdev_t *sdev) {
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

int stratis_svolume_list(svolume_list_t **svolume_list) {
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
 * Devices
 */

/*
 * Question, do we want to have a representation of a device or
 * should we just pass the name of the device?
 */
int stratis_sdev_create(sdev_t **sdev,
			char *name, stratis_dev_t type) {
	int rc = STRATIS_OK;
	sdev_t *return_sdev;

	return_sdev = malloc(sizeof(sdev_t));

    if (return_sdev == NULL)
    	return STRATIS_MALLOC;

    strncpy(return_sdev->name, name, MAX_STRATIS_NAME_LEN);
    return_sdev->id = dev_id++;
    return_sdev->type = type;

    *sdev = return_sdev;

out:
	return rc;
}

int stratis_sdev_destroy(sdev_t *sdev) {
	int rc = STRATIS_OK;

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

int stratis_sdev_list_add(sdev_list_t **sdev_list, sdev_t *sdev) {
	int rc = STRATIS_OK;

	if (sdev_list == NULL || *sdev_list == NULL || sdev == NULL)
		return STRATIS_NULL;

	(*sdev_list)->list =  g_list_append ((*sdev_list)->list, sdev);

	return rc;
}

int stratis_sdev_list_remove(sdev_list_t **sdev_list, sdev_t *sdev) {
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
				sdev_t **sdev,
				int element) {

	int rc = STRATIS_OK;

    if (sdev_list == NULL || element < 0)
    	return STRATIS_NULL;

    *sdev = g_list_nth_data(sdev_list->list, element);

    return rc;
}

static int util_create_disk_list(sdev_list_t **dev_list) {
	int rc = EXIT_SUCCESS;
	int i;
	sdev_t *sdev;
	int size;
	stratis_dev_t type;
	char name[MAX_STRATIS_NAME_LEN];

	rc = stratis_sdev_list_create(dev_list);

	if (rc != STRATIS_OK) {
		FAIL(rc, out, "stratis_sdev_list_create(): rc != 0\n");
	}

	for (i = 0; i < TEST_DEV_COUNT; i++) {

		if (i % 5 == 0)
			type = STRATIS_DEV_TYPE_REGULAR;
		else
			type = STRATIS_DEV_TYPE_CACHE;
	    snprintf(name, MAX_STRATIS_NAME_LEN, "/dev/sdev%d", i);

		rc = stratis_sdev_create(&sdev, name, type);

		if (rc != STRATIS_OK) {
			FAIL(rc, out, "stratis_sdev_create(): rc != 0\n");
		}

		rc = stratis_sdev_list_add(dev_list, sdev);

		if (rc != STRATIS_OK) {
			FAIL(rc, out, "stratis_sdev_list_add(): rc != 0\n");
		}
	}

	rc = stratis_sdev_list_size(*dev_list, &size);

	if (size != TEST_DEV_COUNT){
		FAIL(rc, out, "list size incorrect : size != TEST_DEV_COUNT\n");
	}

out:
	return rc;
}

int populate_simulator_test_data() {
	int rc = EXIT_SUCCESS;
	sdev_list_t *dev_list;
	spool_t *spool;
	svolume_t *svolume;
	sdev_t *sdev;
	char spool_name[256], svolume_name[256], mount_point[256], sdev_name[256];
	struct stratis_ctx *ctx = NULL;
	int i, j, k;

	rc = stratis_context_new(&ctx);

	if (rc != STRATIS_OK) {
		FAIL(rc, out, "stratis_context_new(): rc != 0\n");
	}

	for (i = 0; i < TEST_POOL_COUNT; i++) {
		rc = util_create_disk_list(&dev_list);

		if (rc != STRATIS_OK) {
			FAIL(rc, out, "util_create_disk_list(): rc != 0\n");
		}

		snprintf(spool_name, 256, "stratis_pool%d", i);

		rc = stratis_spool_create(&spool,
				spool_name,
				dev_list,
				STRATIS_VOLUME_RAID_TYPE_RAID4);

		if (rc != STRATIS_OK) {
			FAIL(rc, out, "stratis_spool_create(): rc != 0\n");
		}

		for (j = 0; j < TEST_VOLUME_COUNT; j++) {
			snprintf(svolume_name, 256, "stratis_volume%d", i);
			snprintf(mount_point, 256,"/dev/abc%d", i);
			rc = stratis_svolume_create(&svolume, spool, svolume_name, mount_point);

			if (rc != STRATIS_OK) {
				FAIL(rc, out, "stratis_svolume_create(): rc != 0\n");
			}


		}

		for (j = 0; j < TEST_DEV_COUNT; j++) {
			snprintf(sdev_name, 256, "stratis_dev%d", i);

			rc = stratis_sdev_create(&sdev, sdev_name, STRATIS_DEV_TYPE_REGULAR);

			if (rc != STRATIS_OK) {
				FAIL(rc, out, "stratis_sdev_create(): rc != 0\n");
			}

			rc =  stratis_spool_add_dev(spool, sdev);

			if (rc != STRATIS_OK) {
				FAIL(rc, out, "stratis_spool_add_dev(): rc != 0\n");
			}
		}

	}

out:
	return rc;

}
