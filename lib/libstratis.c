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
#include <stddef.h>
#include <stdarg.h>
#include <unistd.h>
#include <errno.h>
#include <string.h>
#include <ctype.h>
#include <sys/syslog.h>

#include "stratis/libstratis.h"
#include "libstratis-private.h"

static int dbus_id = 0;

/**
 * SECTION:libstratis
 * @short_description: libstratis context
 *
 * The context contains the default values for the library user,
 * and is passed to all library operations.
 */

/**
 * stratis_ctx:
 *
 * Opaque object representing the library context.
 */
struct stratis_ctx {
	int refcount;
	void (*log_fn)(struct stratis_ctx *ctx, int priority, const char *file,
	        int line, const char *fn, const char *format, va_list args);
	void *userdata;
	int log_priority;
	spool_table_t *spool_table;
};

void stratis_log(struct stratis_ctx *ctx, int priority, const char *file,
        int line, const char *fn, const char *format, ...) {
	va_list args;

	va_start(args, format);
	ctx->log_fn(ctx, priority, file, line, fn, format, args);
	va_end(args);
}

static void log_stderr(struct stratis_ctx *ctx, int priority, const char *file,
        int line, const char *fn, const char *format, va_list args) {
	fprintf(stderr, "libstratis: %s: ", fn);
	vfprintf(stderr, format, args);
}

/**
 * stratis_get_userdata:
 * @ctx: abc library context
 *
 * Retrieve stored data pointer from library context. This might be useful
 * to access from callbacks like a custom logging function.
 *
 * Returns: stored userdata
 **/
STRATIS_EXPORT void *stratis_get_userdata(struct stratis_ctx *ctx) {
	if (ctx == NULL)
		return NULL;
	return ctx->userdata;
}

/**
 * stratis_set_userdata:
 * @ctx: abc library context
 * @userdata: data pointer
 *
 * Store custom @userdata in the library context.
 **/
STRATIS_EXPORT void stratis_set_userdata(struct stratis_ctx *ctx,
        void *userdata) {
	if (ctx == NULL)
		return;
	ctx->userdata = userdata;
}

static int log_priority(const char *priority) {
	char *endptr;
	int prio;

	prio = strtol(priority, &endptr, 10);
	if (endptr[0] == '\0' || isspace(endptr[0]))
		return prio;
	if (strncmp(priority, "err", 3) == 0)
		return LOG_ERR;
	if (strncmp(priority, "info", 4) == 0)
		return LOG_INFO;
	if (strncmp(priority, "debug", 5) == 0)
		return LOG_DEBUG;
	return 0;
}

/**
 * stratis_new:
 *
 * Create stratis library context. This reads the stratis configuration
 * and fills in the default values.
 *
 * The initial refcount is 1, and needs to be decremented to
 * release the resources of the stratis library context.
 *
 * Returns: a new stratis library context
 **/
STRATIS_EXPORT int stratis_context_new(struct stratis_ctx **ctx) {
	const char *env;
	struct stratis_ctx *c;

	c = calloc(1, sizeof(struct stratis_ctx));
	if (!c)
		return -ENOMEM;

	c->refcount = 1;
	c->log_fn = log_stderr;
	c->log_priority = LOG_ERR;

	/* environment overwrites config */
	env = secure_getenv("STRATIS_LOG");
	if (env != NULL)
		stratis_set_log_priority(c, log_priority(env));

	info(c, "ctx %p created\n", c);
	dbg(c, "log_priority=%d\n", c->log_priority);
	*ctx = c;

	c->spool_table = malloc(sizeof(spool_table_t));
	c->spool_table->table = g_hash_table_new (g_str_hash, g_str_equal);

	return STRATIS_OK;
}

/**
 * stratis_ref:
 * @ctx: abc library context
 *
 * Take a reference of the abc library context.
 *
 * Returns: the passed abc library context
 **/
STRATIS_EXPORT struct stratis_ctx *stratis_ref(struct stratis_ctx *ctx) {
	if (ctx == NULL)
		return NULL;
	ctx->refcount++;
	return ctx;
}

/**
 * stratis_unref:
 * @ctx: abc library context
 *
 * Drop a reference of the abc library context.
 *
 **/
STRATIS_EXPORT struct stratis_ctx *stratis_unref(struct stratis_ctx *ctx) {
	if (ctx == NULL)
		return NULL;
	ctx->refcount--;
	if (ctx->refcount > 0)
		return NULL;
	info(ctx, "context %p released\n", ctx);
	free(ctx);
	return NULL;
}

/**
 * stratis_set_log_fn:
 * @ctx: abc library context
 * @log_fn: function to be called for logging messages
 *
 * The built-in logging writes to stderr. It can be
 * overridden by a custom function, to plug log messages
 * into the user's logging functionality.
 *
 **/
STRATIS_EXPORT void stratis_set_log_fn(struct stratis_ctx *ctx,
        void (*log_fn)(struct stratis_ctx *ctx, int priority, const char *file,
                int line, const char *fn, const char *format, va_list args)) {
	ctx->log_fn = log_fn;
	info(ctx, "custom logging function %p registered\n", log_fn);
}

/**
 * stratis_get_log_priority:
 * @ctx: abc library context
 *
 * Returns: the current logging priority
 **/
STRATIS_EXPORT int stratis_get_log_priority(struct stratis_ctx *ctx) {
	return ctx->log_priority;
}

/**
 * stratis_set_log_priority:
 * @ctx: abc library context
 * @priority: the new logging priority
 *
 * Set the current logging priority. The value controls which messages
 * are logged.
 **/
STRATIS_EXPORT void stratis_set_log_priority(struct stratis_ctx *ctx,
        int priority) {
	ctx->log_priority = priority;
}

struct stratis_list_entry;

struct stratis_thing {
	struct stratis_ctx *ctx;
	int refcount;
};

STRATIS_EXPORT struct stratis_thing *stratis_thing_ref(
        struct stratis_thing *thing) {
	if (!thing)
		return NULL;
	thing->refcount++;
	return thing;
}

STRATIS_EXPORT struct stratis_thing *stratis_thing_unref(
        struct stratis_thing *thing) {
	if (thing == NULL)
		return NULL;
	thing->refcount--;
	if (thing->refcount > 0)
		return NULL;
	dbg(thing->ctx, "context %p released\n", thing);
	stratis_unref(thing->ctx);
	free(thing);
	return NULL;
}

STRATIS_EXPORT struct stratis_ctx *stratis_thing_get_ctx(
        struct stratis_thing *thing) {
	return thing->ctx;
}

STRATIS_EXPORT struct stratis_list_entry *stratis_thing_get_some_list_entry(
        struct stratis_thing *thing) {
	return NULL;
}

STRATIS_EXPORT char * stratis_get_raid_token(int stratis_code) {
	switch (stratis_code) {
		case STRATIS_RAID_TYPE_UNKNOWN:
			return "STRATIS_RAID_TYPE_UNKNOWN";
		case STRATIS_RAID_TYPE_SINGLE:
			return "STRATIS_RAID_TYPE_SINGLE";
		case STRATIS_RAID_TYPE_RAID1:
			return "STRATIS_RAID_TYPE_RAID1";
		case STRATIS_RAID_TYPE_RAID5:
			return "STRATIS_RAID_TYPE_RAID5";
		case STRATIS_RAID_TYPE_RAID6:
			return "STRATIS_RAID_TYPE_RAID6";
		case STRATIS_RAID_TYPE_MAX:
			return "STRATIS_RAID_TYPE_MAX";
	}
	return "STRATIS_RAID_TYPE_UNKNOWN";
}

STRATIS_EXPORT char * stratis_get_dev_type_token(int stratis_code) {
	switch (stratis_code) {
		case STRATIS_DEV_TYPE_UNKNOWN:
			return "STRATIS_DEV_TYPE_UNKNOWN";
		case STRATIS_DEV_TYPE_REGULAR:
			return "STRATIS_DEV_TYPE_REGULAR";
		case STRATIS_DEV_TYPE_CACHE:
			return "STRATIS_DEV_TYPE_CACHE";
		case STRATIS_DEV_TYPE_SPARE:
			return "STRATIS_DEV_TYPE_SPARE";
		case STRATIS_DEV_TYPE_MAX:
			return "STRATIS_DEV_TYPE_MAX";
	}

	return "STRATIS_DEV_TYPE_UNKNOWN";
}

STRATIS_EXPORT char * stratis_get_code_token(int stratis_code) {
	switch (stratis_code) {
		case STRATIS_OK:
			return "STRATIS_OK";
		case STRATIS_ERROR:
			return "STRATIS_ERROR";
		case STRATIS_NULL:
			return "STRATIS_NULL";
		case STRATIS_MALLOC:
			return "STRATIS_MALLOC";
		case STRATIS_NOTFOUND:
			return "STRATIS_NOTFOUND";
		case STRATIS_POOL_NOTFOUND:
			return "STRATIS_POOL_NOTFOUND";
		case STRATIS_VOLUME_NOTFOUND:
			return "STRATIS_VOLUME_NOTFOUND";
		case STRATIS_BAD_PARAM:
			return "STRATIS_BAD_PARAM";
		case STRATIS_ALREADY_EXISTS:
			return "STRATIS_ALREADY_EXISTS";
		case STRATIS_DUPLICATE_NAME:
			return "STRATIS_DUPLICATE_NAME";
		case STRATIS_NO_POOLS:
			return "STRATIS_NO_POOLS";
		case STRATIS_LIST_FAILURE:
			return "STRATIS_LIST_FAILURE";
	}

	return "UNKNOWN_CODE";
}

STRATIS_EXPORT char * stratis_raid_user_message(int stratis_code) {
	switch (stratis_code) {
		case STRATIS_RAID_TYPE_UNKNOWN:
			return "<unknown>";
		case STRATIS_RAID_TYPE_SINGLE:
			return "<single user description>";
		case STRATIS_RAID_TYPE_RAID1:
			return "<raid1 user description>";
		case STRATIS_RAID_TYPE_RAID5:
			return "<raid5 user description>";
		case STRATIS_RAID_TYPE_RAID6:
			return "<raid6 user description>";
		case STRATIS_RAID_TYPE_MAX:
			return "STRATIS_RAID_TYPE_MAX";
	}
	return "STRATIS_RAID_TYPE_UNKNOWN";
}

STRATIS_EXPORT char * stratis_get_dev_type_message(int stratis_code) {
	switch (stratis_code) {
		case STRATIS_DEV_TYPE_UNKNOWN:
			return "<unknown type user description>";
		case STRATIS_DEV_TYPE_REGULAR:
			return "<dev user description>";
		case STRATIS_DEV_TYPE_CACHE:
			return "<cache user description>";
		case STRATIS_DEV_TYPE_SPARE:
			return "<spare user description>";
		case STRATIS_DEV_TYPE_MAX:
			return "STRATIS_DEV_TYPE_MAX";
	}

	return "STRATIS_DEV_TYPE_UNKNOWN";
}

STRATIS_EXPORT char * stratis_get_user_message(int stratis_code) {

	switch (stratis_code) {
		case STRATIS_OK:
			return "ok";
		case STRATIS_ERROR:
			return "error";
		case STRATIS_NULL:
			return "NULL parameter";
		case STRATIS_MALLOC:
			return "malloc failed";
		case STRATIS_NOTFOUND:
			return "not found";
		case STRATIS_POOL_NOTFOUND:
			return "pool not found";
		case STRATIS_VOLUME_NOTFOUND:
			return "volume not found";
		case STRATIS_DEV_NOTFOUND:
			return "dev not found";
		case STRATIS_CACHE_NOTFOUND:
			return "cache not found";
		case STRATIS_BAD_PARAM:
			return "bad parameter";
		case STRATIS_ALREADY_EXISTS:
			return "already exists";
		case STRATIS_DUPLICATE_NAME:
			return "duplicate name";
		case STRATIS_NO_POOLS:
			return "no pools";
		case STRATIS_LIST_FAILURE:
			return "list transaction failure";
	}

	return "unknown error";
}


STRATIS_EXPORT int stratis_cache_get(struct stratis_ctx *ctx, scache_t **scache, char *name) {
	GList *values;
	int list_size, i;
	int rc = STRATIS_OK;
	spool_t *spool;

	if (scache == NULL) {
		rc = STRATIS_MALLOC;
		goto out;
	}

	if (ctx->spool_table == NULL || ctx->spool_table->table == NULL) {
		rc = STRATIS_DEV_NOTFOUND;
		goto out;
	}

	values = g_hash_table_get_values(ctx->spool_table->table);
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

STRATIS_EXPORT int stratis_sdev_get(struct stratis_ctx *ctx, sdev_t **sdev, char *name) {
	GList *values;
	int list_size, i;
	int rc = STRATIS_OK;
	spool_t *spool;

	if (sdev == NULL) {
		rc = STRATIS_MALLOC;
		goto out;
	}

	if (ctx->spool_table->table == NULL || ctx->spool_table->table == NULL) {
		rc = STRATIS_DEV_NOTFOUND;
		goto out;
	}

	values = g_hash_table_get_values(ctx->spool_table->table);
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

STRATIS_EXPORT int stratis_spool_create(struct stratis_ctx *ctx, spool_t **spool, const char *name,
        sdev_table_t *disk_list, int raid_level) {
	int rc = STRATIS_OK;
	spool_t *return_spool = NULL;

	return_spool = malloc(sizeof(spool_t));

	if (return_spool == NULL) {
		rc = STRATIS_MALLOC;
		goto out;
	}

	rc = stratis_svolume_table_create(&(return_spool->svolume_table));

	if (rc != STRATIS_OK)
		goto out;

	rc = stratis_sdev_table_create(&(return_spool->sdev_table));

	if (rc != STRATIS_OK)
		goto out;

	rc = stratis_scache_table_create(&(return_spool->scache_table));

	if (rc != STRATIS_OK)
		goto out;

	return_spool->slot = NULL;
	return_spool->id = dbus_id++;
	return_spool->size = 32767;
	strncpy(return_spool->name, name, MAX_STRATIS_NAME_LEN);

	/* TODO should we duplicate the disk_list? */
	return_spool->sdev_table = disk_list;

	g_hash_table_insert(ctx->spool_table->table, return_spool->name, return_spool);

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

STRATIS_EXPORT int stratis_spool_destroy(struct stratis_ctx *ctx, spool_t *spool) {
	int rc = STRATIS_OK;

	if (spool == NULL) {
		rc = STRATIS_NULL;
		goto out;
	}
	gboolean found = g_hash_table_remove(ctx->spool_table->table, spool->name);

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

STRATIS_EXPORT int stratis_spool_get(struct stratis_ctx *ctx, spool_t **spool, char *name) {
	int rc = STRATIS_OK;

	if (spool == NULL || ctx->spool_table == NULL) {
		return STRATIS_NULL;
	}

	*spool = g_hash_table_lookup(ctx->spool_table->table, name);

	if (*spool == NULL)
		rc = STRATIS_NOTFOUND;

	return rc;
}

STRATIS_EXPORT char *stratis_spool_get_name(spool_t *spool) {

	if (spool == NULL) {
		return NULL;
	}

	return spool->name;
}


STRATIS_EXPORT int stratis_spool_get_id(spool_t *spool) {

	if (spool == NULL) {
		return -1;
	}

	return spool->id;
}

STRATIS_EXPORT int stratis_spool_get_list(struct stratis_ctx *ctx, spool_table_t **spool_list) {

	if (ctx == NULL || spool_list == NULL)
		return STRATIS_NULL;

	*spool_list = ctx->spool_table;

	return STRATIS_OK;
}

STRATIS_EXPORT int stratis_spool_get_volume_list(spool_t *spool, svolume_table_t **svolume_list) {

	int rc = STRATIS_OK;

	if (spool == NULL || svolume_list == NULL)
		return STRATIS_NULL;

	*svolume_list = spool->svolume_table;

	return rc;
}

STRATIS_EXPORT int stratis_spool_get_dev_table(spool_t *spool, sdev_table_t **sdev_table) {

	int rc = STRATIS_OK;

	if (spool == NULL || sdev_table == NULL)
		return STRATIS_NULL;

	*sdev_table = spool->sdev_table;

	return rc;
}

STRATIS_EXPORT int stratis_spool_add_volume(spool_t *spool, svolume_t *volume) {
	int rc = STRATIS_OK;
	svolume_t *ptr;

	if (spool == NULL || spool->svolume_table == NULL|| volume == NULL)
		return STRATIS_NULL;

	if (strlen(volume->name) == 0)
		return STRATIS_NULL_NAME;

	ptr =  g_hash_table_lookup(spool->svolume_table->table, volume->name);
	if (ptr != NULL)
		return STRATIS_ALREADY_EXISTS;

	g_hash_table_insert(spool->svolume_table->table, volume->name, volume);

	return rc;
}


STRATIS_EXPORT int stratis_spool_add_dev(spool_t *spool, sdev_t *sdev) {
	int inserted = FALSE;
	sdev_t *ptr;
	if (spool == NULL || sdev == NULL || spool->scache_table == NULL)
		return STRATIS_NULL;

	if (strlen(sdev->name) == 0)
		return STRATIS_NULL_NAME;

	ptr =  g_hash_table_lookup(spool->sdev_table->table, sdev->name);
	if (ptr != NULL)
		return STRATIS_ALREADY_EXISTS;

	inserted =  g_hash_table_insert(spool->sdev_table->table, sdev->name, sdev);

	if (inserted == FALSE)
		return STRATIS_ALREADY_EXISTS;
	else
		return STRATIS_OK;
}

STRATIS_EXPORT int stratis_spool_add_cache(spool_t *spool, scache_t *scache) {
	int inserted = FALSE;
	scache_t *ptr;

	if (spool == NULL || scache == NULL || spool->sdev_table == NULL)
		return STRATIS_NULL;

	if (strlen(scache->name) == 0)
		return STRATIS_NULL_NAME;

	ptr =  g_hash_table_lookup(spool->scache_table->table, scache->name);
	if (ptr != NULL)
		return STRATIS_ALREADY_EXISTS;

	inserted = g_hash_table_insert(spool->scache_table->table, scache->name, scache);

	if (inserted == FALSE)
		return STRATIS_ERROR;
	else
		return STRATIS_OK;
}

static void
iterate_dev_remove (gpointer key, gpointer value, gpointer user_data)
{
	GHashTable *table = user_data;

	g_hash_table_remove(table, key);

}

STRATIS_EXPORT int stratis_spool_remove_cache_devs(spool_t *spool, sdev_table_t *scache_table) {

	g_hash_table_foreach(scache_table->table, iterate_dev_remove,
			spool->sdev_table->table);

	return STRATIS_OK;
}

STRATIS_EXPORT int stratis_spool_remove_devs(spool_t *spool, sdev_table_t *sdev_table) {

	g_hash_table_foreach(sdev_table->table, iterate_dev_remove,
			spool->sdev_table->table);

	return STRATIS_OK;
}

STRATIS_EXPORT int stratis_spool_remove_dev(spool_t *spool, char *name) {

	int removed, rc;

	removed = g_hash_table_remove(spool->sdev_table->table, name);

	if (removed == TRUE) {
		rc = STRATIS_OK;
	} else {
		rc = STRATIS_DEV_NOTFOUND;
	}
	return rc;
}

STRATIS_EXPORT int stratis_spool_get_cache_dev_table(spool_t *spool, scache_table_t **scache_table) {
	if (spool == NULL || spool->sdev_table == NULL)
		return STRATIS_NULL;

	// TODO make copy
	*scache_table = spool->scache_table;

	return STRATIS_OK;
}


STRATIS_EXPORT int stratis_spool_table_find(spool_table_t *spool_table, spool_t **spool,
        char *name) {
	GHashTable *l;
	if (spool == NULL || spool_table == NULL)
		return STRATIS_NULL;

	*spool = g_hash_table_lookup(spool_table->table, name);

	if (*spool == NULL)
		return STRATIS_NOTFOUND;

	return STRATIS_OK;

}

STRATIS_EXPORT int stratis_spool_list_size(spool_table_t *spool_list, int *list_size) {
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
STRATIS_EXPORT int stratis_svolume_create(svolume_t **svolume, spool_t *spool, char *name,
        	char *mount_point, char *quota) {
	int rc = STRATIS_OK;

	svolume_t *return_volume;

	return_volume = malloc(sizeof(svolume_t));

	if (return_volume == NULL)
		return STRATIS_MALLOC;

	strncpy(return_volume->name, name, MAX_STRATIS_NAME_LEN);
	strncpy(return_volume->mount_point, (mount_point == NULL ? "" : mount_point), MAX_STRATIS_NAME_LEN);
	strncpy(return_volume->quota, (quota == NULL ? "" : mount_point), MAX_STRATIS_NAME_LEN);
	return_volume->id = dbus_id++;
	return_volume->parent_spool = spool;


	return_volume->dbus_name[0] = '\0';
	rc = stratis_spool_add_volume(spool, return_volume);

	if (rc != STRATIS_OK)
		goto out;

	*svolume = return_volume;

	out: return rc;
}

STRATIS_EXPORT int stratis_svolume_destroy(svolume_t *svolume) {
	int removed, rc;

	if (svolume == NULL || svolume->parent_spool == NULL ||
			svolume->parent_spool->svolume_table == NULL) {
		return STRATIS_NULL;
	}

	removed = g_hash_table_remove(svolume->parent_spool->svolume_table->table,
			svolume->name);

	if (removed == TRUE) {
		rc = STRATIS_OK;
	} else {
		rc = STRATIS_VOLUME_NOTFOUND;
	}
	return rc;
}

STRATIS_EXPORT int stratis_svolume_get(struct stratis_ctx *ctx, svolume_t **svolume, char *poolname, char *volumename) {
	int rc = STRATIS_OK;
	spool_t *spool = NULL;

	if (svolume == NULL || ctx->spool_table == NULL ||
			poolname == NULL || volumename == NULL  ) {
		return STRATIS_NULL;
	}

	spool = g_hash_table_lookup(ctx->spool_table->table, poolname);

    if (spool == NULL)
        return STRATIS_POOL_NOTFOUND;

    if (spool->svolume_table == NULL)
        return STRATIS_VOLUME_NOTFOUND;

	*svolume = g_hash_table_lookup(spool->svolume_table->table, volumename);

	return rc;
}

STRATIS_EXPORT  char *stratis_svolume_get_name(svolume_t *svolume) {

	if (svolume == NULL) {
		return NULL;
	}

	return svolume->name;
}
STRATIS_EXPORT int stratis_svolume_set_quota(svolume_t *svolume, char *quota) {

	if (svolume == NULL || quota == NULL)
		return STRATIS_NULL;

	strncpy(svolume->quota, quota, MAX_STRATIS_NAME_LEN);
	svolume->quota[MAX_STRATIS_NAME_LEN] = '\0';

	return STRATIS_OK;
}

STRATIS_EXPORT int stratis_svolume_set_mount_point(svolume_t *svolume, char *mount_point) {

	if (svolume == NULL || mount_point == NULL)
		return STRATIS_NULL;

	strncpy(svolume->mount_point, mount_point, MAX_STRATIS_NAME_LEN);
	svolume->mount_point[MAX_STRATIS_NAME_LEN] = '\0';

	return STRATIS_OK;
}

STRATIS_EXPORT int stratis_svolume_rename(svolume_t *svolume, char *name) {

	if (svolume == NULL || name == NULL)
		return STRATIS_NULL;

	g_hash_table_remove(svolume->parent_spool->svolume_table->table,
			svolume->name);

	strncpy(svolume->name, name, MAX_STRATIS_NAME_LEN);
	svolume->name[MAX_STRATIS_NAME_LEN] = '\0';

	g_hash_table_insert(svolume->parent_spool->svolume_table->table,
			svolume->name, svolume);

	return STRATIS_OK;
}

STRATIS_EXPORT int stratis_svolume_get_id(svolume_t *svolume) {

	if (svolume == NULL) {
		return -1;
	}

	return svolume->id;
}

STRATIS_EXPORT char *stratis_svolume_get_mount_point(svolume_t *svolume) {
	if (svolume == NULL) {
		return NULL;
	}

	return svolume->mount_point;
}

STRATIS_EXPORT int stratis_svolume_table_find(svolume_table_t *svolume_table, svolume_t **svolume,
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


STRATIS_EXPORT int stratis_svolume_table_create(svolume_table_t **svolume_table) {
	int rc = STRATIS_OK;

	if (svolume_table == NULL) {
		rc = STRATIS_NULL;
		goto out;
	}

	*svolume_table = malloc(sizeof(svolume_table_t));

	if (*svolume_table == NULL) {
		rc = STRATIS_MALLOC;
		goto out;
	}

	(*svolume_table)->table = g_hash_table_new (g_str_hash, g_str_equal);

out:
	return rc;
}

STRATIS_EXPORT int stratis_svolume_table_destroy(svolume_table_t *svolume_table) {
	int rc = STRATIS_OK;

	return rc;
}

STRATIS_EXPORT int stratis_svolume_table_size(svolume_table_t *svolume_table, int *list_size) {
	int rc = STRATIS_OK;

	if (svolume_table == NULL || list_size == NULL)
		return STRATIS_NULL;

	if (svolume_table->table == NULL)
		*list_size = 0;
	else
		*list_size = g_hash_table_size(svolume_table->table);

	return rc;
}

STRATIS_EXPORT int stratis_svolume_create_snapshot(svolume_t *svolume,
		spool_t *spool, svolume_t **snapshot, char *name) {
	int rc = STRATIS_OK;

	if (svolume == NULL || spool->svolume_table == NULL
			|| svolume == NULL || spool->svolume_table->table == NULL
			|| snapshot == NULL)
		return STRATIS_NULL;

	if (strlen(svolume->name) == 0)
		return STRATIS_NULL_NAME;

	*snapshot =  g_hash_table_lookup(spool->svolume_table->table, name);
	if (*snapshot != NULL)
		return STRATIS_ALREADY_EXISTS;


	rc = stratis_svolume_create(snapshot, spool, name, NULL, NULL);
	(*snapshot)->parent_volume = svolume;

	g_hash_table_insert(spool->svolume_table->table, (*snapshot)->name, snapshot);

	return rc;

}


/*
 * Devs
 */
STRATIS_EXPORT int stratis_sdev_create(sdev_t **sdev, spool_t *spool,
			char *name, int type) {
	int rc = STRATIS_OK;
	sdev_t *return_sdev;

	if (sdev == NULL || name == NULL) {
		rc = STRATIS_NULL;
		goto out;
	}
	return_sdev = malloc(sizeof(sdev_t));
	if (return_sdev == NULL) {
		rc = STRATIS_MALLOC;
	}

	strncpy(return_sdev->name, name, MAX_STRATIS_NAME_LEN);

	return_sdev->id = dbus_id++;
	return_sdev->parent_spool = spool;

	*sdev = return_sdev;
out:
	return rc;
}



STRATIS_EXPORT char *stratis_sdev_get_name(sdev_t *sdev) {

	if (sdev == NULL) {
		return NULL;
	}

	return sdev->name;
}

STRATIS_EXPORT int stratis_sdev_get_id(sdev_t *sdev) {

	if (sdev == NULL) {
		return -1;
	}

	return sdev->id;
}

/*
 * Cache
 */

/*
 * Device Lists
 */
STRATIS_EXPORT int stratis_sdev_table_create(sdev_table_t **sdev_table) {
	int rc = STRATIS_OK;
	sdev_table_t *return_sdev_list;

	return_sdev_list = malloc(sizeof(sdev_table_t));
	if (return_sdev_list == NULL)
		return STRATIS_MALLOC;

	return_sdev_list->table = g_hash_table_new (g_str_hash, g_str_equal);

	*sdev_table = return_sdev_list;
	return rc;
}

STRATIS_EXPORT int stratis_sdev_table_destroy(sdev_table_t *sdev_table) {
	int rc = STRATIS_OK;

	return rc;
}

STRATIS_EXPORT int stratis_sdev_table_add(sdev_table_t *sdev_table, sdev_t *sdev) {
	int rc = STRATIS_OK;
	char *list_copy = NULL;

	if (sdev_table == NULL ||  sdev == NULL)
		return STRATIS_NULL;

	g_hash_table_insert(sdev_table->table, sdev->name, sdev);

	return rc;
}

STRATIS_EXPORT int stratis_sdev_table_size(sdev_table_t *sdev_table, int *table_size) {
	int rc = STRATIS_OK;

	if (sdev_table == NULL || table_size == NULL)
		return STRATIS_NULL;

	if (sdev_table->table == NULL)
		*table_size = 0;
	else
		*table_size = g_hash_table_size(sdev_table->table);


	return rc;
}

STRATIS_EXPORT int stratis_sdev_table_remove(sdev_table_t **sdev_table, char *sdev) {
	int rc = STRATIS_OK;

	return rc;
}

STRATIS_EXPORT int stratis_scache_table_create(scache_table_t **scache_table) {
	int rc = STRATIS_OK;
	scache_table_t *return_sdev_list;

	return_sdev_list = malloc(sizeof(sdev_table_t));
	if (return_sdev_list == NULL)
		return STRATIS_MALLOC;

	return_sdev_list->table = g_hash_table_new (g_str_hash, g_str_equal);

	*scache_table = return_sdev_list;
	return rc;
}

STRATIS_EXPORT int stratis_scache_table_destroy(scache_table_t *scache_table) {
	int rc = STRATIS_OK;

	return rc;
}

STRATIS_EXPORT int stratis_scache_create(scache_t **scache, spool_t *spool,char *name, int type) {
	scache_t *return_scache;
	int rc = STRATIS_OK;

	if (scache == NULL || name == NULL) {
		rc = STRATIS_NULL;
		goto out;
	}
	return_scache = malloc(sizeof(scache_t));
	if (return_scache == NULL) {
		rc = STRATIS_MALLOC;
	}

	strncpy(return_scache->name, name, MAX_STRATIS_NAME_LEN);

	return_scache->id = dbus_id++;
	return_scache->parent_spool = spool;

	*scache = return_scache;
out:
	return rc;
}

STRATIS_EXPORT char *stratis_scache_get_name(scache_t *scache) {

	if (scache == NULL) {
		return NULL;
	}

	return scache->name;
}

STRATIS_EXPORT int stratis_scache_get_id(scache_t *scache) {
	if (scache == NULL) {
		return -1;
	}

	return scache->id;
}
