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
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <dlfcn.h>
#include <glib.h>
#include <semaphore.h>
#include <errno.h>
#include <pthread.h>
#include <sys/types.h>
#include <sys/select.h>
#include <sys/socket.h>
#include <microhttpd.h>

#include "stratis-common.h"
#include "libstratis.h"
#include "test.h"

#define TEST_DEV_COUNT 20
#define TEST_POOL_COUNT 10
#define TEST_VOLUME_COUNT 5

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

static int test_stratis_pool_creation() {
	int rc = EXIT_SUCCESS;
	sdev_list_t *dev_list;
	spool_t *spool;
	svolume_t *svolume;
	struct stratis_ctx *ctx = NULL;
	int i, j;

	rc = stratis_context_new(&ctx);

	if (rc != STRATIS_OK) {
		FAIL(rc, out, "stratis_context_new(): rc != 0\n");
	}

	for (i = 0; i < TEST_POOL_COUNT; i++) {
		rc = util_create_disk_list(&dev_list);

		if (rc != STRATIS_OK) {
			FAIL(rc, out, "util_create_disk_list(): rc != 0\n");
		}

		rc = stratis_spool_create(&spool, dev_list, STRATIS_VOLUME_RAID_TYPE_RAID4);

		if (rc != STRATIS_OK) {
			FAIL(rc, out, "stratis_spool_create(): rc != 0\n");
		}

		for (j = 0; j < TEST_VOLUME_COUNT; j++) {
			rc = stratis_svolume_create(&svolume, spool, "volume", "/dev/abc");

			if (rc != STRATIS_OK) {
				FAIL(rc, out, "stratis_svolume_create(): rc != 0\n");
			}
		}

	}

out:
	return rc;

}
int main(int argc, char **argv) {
    int rc = EXIT_SUCCESS;

    rc = test_stratis_pool_creation();

    exit(rc);
}
