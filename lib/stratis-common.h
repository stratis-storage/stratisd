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

#ifndef STRATIS_COMMON_H
#define STRATIS_COMMON_H

#define FAIL(rc, out, ...) \
        do { \
                rc = EXIT_FAILURE; \
                fprintf(stderr, "FAIL: "__VA_ARGS__ ); \
                goto out; \
        } while(0)
#define PASS(...) fprintf(stdout, "PASS: "__VA_ARGS__ );

#define DBUS_MAX_NAME 255

#define STRATIS_VERSION "1"
#define MANAGER_NAME "/Manager"
#define STRATIS_BASE_PATH "/org/storage/stratis" STRATIS_VERSION
#define STRATIS_BASE_SERVICE "org.storage.stratis" STRATIS_VERSION
#define STRATIS_BASE_MANAGER STRATIS_BASE_PATH "/Manager"
#define STRATIS_MANAGER_INTERFACE "org.storage.stratis" STRATIS_VERSION ".Manager"
#define STRATIS_POOL_BASE_INTERFACE "org.storage.stratis" STRATIS_VERSION ".pool"
#define STRATIS_VOLUME_BASE_INTERFACE "org.storage.stratis" STRATIS_VERSION ".volume"
#define STRATIS_DEV_BASE_INTERFACE "org.storage.stratis" STRATIS_VERSION ".dev"
#define STRATIS_POOL_BASE_PATH STRATIS_BASE_PATH "/pool"

/* Volume Property Definitions */
#define VOLUME_NAME 		"Volume"
#define VOLUME_ID	 		"VolumeId"
#define VOLUME_MOUNT_POINT	"MountPoint"
#define VOLUME_QUOTA		"Quota"


/* Pool Property Definitions */
#define POOL_NAME 			"SPool"
#define POOL_ID 			"SPoolId"

/* Disk Property Definitions */
#define DEV_NAME 			"Dev"
#define DEV_ID 				"DevId"
#define DEV_TYPE 			"DevType"
#define DEV_SIZE 			"DevSize"


void * stratis_main_loop(void * ap);
void quit_stratis_main_loop();

#endif /* STRATIS_COMMON_H */

