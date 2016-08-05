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
	case STRATIS_BAD_PARAM:
		return "bad parameter";
	case STRATIS_ALREADY_EXISTS:
		return "already exists";
	case STRATIS_DUPLICATE_NAME:
		return "duplicate name";
	case STRATIS_NO_POOLS:
		return "no pools";
	}

	return "unknown error";
}

