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

#ifndef LIB_LIBSTRATIS_H_
#define LIB_LIBSTRATIS_H_

#include <glib.h>
#include <stdio.h>
#include <stdlib.h>
#include <systemd/sd-bus.h>
/*
 * stratis_ctx
 *
 * library user context - reads the config and system
 * environment, user variables, allows custom logging
 */
struct stratis_ctx;
struct stratis_ctx *stratis_ref(struct stratis_ctx *ctx);
struct stratis_ctx *stratis_unref(struct stratis_ctx *ctx);
int stratis_context_new(struct stratis_ctx **ctx);
void stratis_set_log_fn(struct stratis_ctx *ctx,
        void (*log_fn)(struct stratis_ctx *ctx, int priority, const char *file,
                int line, const char *fn, const char *format, va_list args));
int stratis_get_log_priority(struct stratis_ctx *ctx);
void stratis_set_log_priority(struct stratis_ctx *ctx, int priority);
void *stratis_get_userdata(struct stratis_ctx *ctx);
void stratis_set_userdata(struct stratis_ctx *ctx, void *userdata);
char * stratis_get_user_message(int stratis_code);

#define MAX_STRATIS_NAME_LEN 256

typedef enum {
	/** Unknown */
	STRATIS_DEV_TYPE_UNKNOWN = -1,
	STRATIS_DEV_TYPE_REGULAR = 0,
	STRATIS_DEV_TYPE_CACHE = 1,
} stratis_dev_t;

typedef struct sdev_list {
	GList *list;
} sdev_list_t;

typedef struct svolume_list {
	GList *list;
} svolume_list_t;

typedef struct spool_list {
	GList *list;
} spool_list_t;

typedef struct spool {
	int id;
	int size;
	char name[MAX_STRATIS_NAME_LEN];
	char dbus_name[MAX_STRATIS_NAME_LEN];
	sd_bus_slot *slot;
	sdev_list_t *sdev_list;
	svolume_list_t *svolume_list;
} spool_t;

typedef struct svolume {
	int id;
	int size;
	spool_t *parent_spool;
	char name[MAX_STRATIS_NAME_LEN];
	char mount_point[MAX_STRATIS_NAME_LEN];
	char quota[MAX_STRATIS_NAME_LEN];
	char dbus_name[MAX_STRATIS_NAME_LEN];
	sd_bus_slot *slot;
} svolume_t;

/* Return codes */
#define STRATIS_OK				0		/* Ok */
#define STRATIS_ERROR			100
#define STRATIS_NULL			101
#define STRATIS_MALLOC			102
#define STRATIS_NOTFOUND		103
#define STRATIS_BAD_PARAM		104
#define STRATIS_ALREADY_EXISTS	105
#define STRATIS_DUPLICATE_NAME	106
#define STRATIS_NO_POOLS		107
/*
 * typedef taken from LSM
 */
typedef enum {
	/** Unknown */
	STRATIS_VOLUME_RAID_TYPE_UNKNOWN = -1,
	/** Single */
	STRATIS_VOLUME_RAID_TYPE_SINGLE = 0,
	/** Mirror between two disks. For 4 disks or more, they are RAID10.*/
	STRATIS_VOLUME_RAID_TYPE_RAID1 = 1,
	/** Block-level striping with distributed parity */
	STRATIS_VOLUME_RAID_TYPE_RAID5 = 5,
	/** Block-level striping with two distributed parities, aka, RAID-DP */
	STRATIS_VOLUME_RAID_TYPE_RAID6 = 6,
} stratis_volume_raid_type;

/*
 * Pools
 */

int stratis_spool_create(spool_t **spool, const char *name,
        sdev_list_t *disk_list, stratis_volume_raid_type raid_level);
int stratis_spool_destroy(spool_t *spool);
char *stratis_spool_get_name(spool_t *spool);
int stratis_spool_get_id(spool_t *spool);
int stratis_spool_get_list(spool_list_t **spool_list);
int stratis_spool_add_devs(spool_t *spool, sdev_list_t *sdev_list);
int stratis_spool_remove_dev(spool_t *spool, char *sdev);
int stratis_spool_get_volume_list(spool_t *spool,
        svolume_list_t **svolume_list);
int stratis_spool_get_dev_list(spool_t *spool, sdev_list_t **sdev_list);
int stratis_spool_list_size(spool_list_t *spool_list, int *list_size);
int stratis_spool_list_nth(spool_list_t *spool_list, spool_t **spool,
        int element);
int stratis_spool_list_find(spool_list_t *spool_list, spool_t **spool,
        char *name);
/*
 * Volumes
 */
int stratis_svolume_create(svolume_t **svolume, spool_t *spool, char *name,
        char *mount_point, char *qutoa);
int stratis_svolume_destroy(svolume_t *svolume);
char *stratis_svolume_get_name(svolume_t *svolume);
int stratis_svolume_get_id(svolume_t *svolume);
char *stratis_svolume_get_mount_point(svolume_t *svolume);

int stratis_svolume_list_create(svolume_list_t **svolume_list);
int stratis_svolume_list_destroy(svolume_list_t *svolume_list);
int stratis_svolume_list_eligible_disks(sdev_list_t **disk_list);
int stratis_svolume_list_devs(spool_t *spool, sdev_list_t **disk_list);
int stratis_svolume_list_size(svolume_list_t *svolume_list, int *list_size);
int stratis_svolume_list_nth(svolume_list_t *svolume_list, svolume_t **svolume,
        int element);
int stratis_svolume_list_find(svolume_list_t *svolume_list, svolume_t **svolume,
        char *name);

/*
 * Device Lists
 */

int stratis_sdev_list_create(sdev_list_t **sdev_list);
int stratis_sdev_list_destroy(sdev_list_t *sdev_list);
int stratis_sdev_list_add(sdev_list_t **sdev_list, char *sdev);
int stratis_sdev_list_remove(sdev_list_t **sdev_list, char *sdev);
int stratis_sdev_list_size(sdev_list_t *sdev_list, int *list_size);
int stratis_sdev_list_nth(sdev_list_t *sdev_list, char **sdev, int element);

/* Simulator */

int populate_simulator_test_data();

#endif /* LIB_LIBSTRATIS_H_ */
