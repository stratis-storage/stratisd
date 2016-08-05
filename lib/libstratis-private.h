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
 *
 */

#ifndef _LIB_STRATIS_PRIVATE_H_
#define _LIB_STRATIS_PRIVATE_H_

#ifdef __cplusplus
extern "C" {
#endif

#include <stdbool.h>
#include <syslog.h>
#include <stdlib.h>

#include <stratis/libstratis.h>

static inline void __attribute__((always_inline, format(printf, 2, 3)))
stratis_log_null(struct stratis_ctx *ctx, const char *format, ...) {
}

#define stratis_log_cond(ctx, prio, arg...) \
  do { \
    if (stratis_get_log_priority(ctx) >= prio) \
      stratis_log(ctx, prio, __FILE__, __LINE__, __FUNCTION__, ## arg); \
  } while (0)

#ifdef ENABLE_LOGGING
#  ifdef ENABLE_DEBUG
#    define dbg(ctx, arg...) stratis_log_cond(ctx, LOG_DEBUG, ## arg)
#  else
#    define dbg(ctx, arg...) stratis_log_null(ctx, ## arg)
#  endif
#  define info(ctx, arg...) stratis_log_cond(ctx, LOG_INFO, ## arg)
#  define err(ctx, arg...) stratis_log_cond(ctx, LOG_ERR, ## arg)
#else
#  define dbg(ctx, arg...) stratis_log_null(ctx, ## arg)
#  define info(ctx, arg...) stratis_log_null(ctx, ## arg)
#  define err(ctx, arg...) stratis_log_null(ctx, ## arg)
#endif

#ifndef HAVE_SECURE_GETENV
#  ifdef HAVE___SECURE_GETENV
#    define secure_getenv __secure_getenv
#  else
#    error neither secure_getenv nor __secure_getenv is available
#  endif
#endif

#define STRATIS_EXPORT __attribute__ ((visibility("default")))

void stratis_log(struct stratis_ctx *ctx, int priority, const char *file,
        int line, const char *fn, const char *format, ...)
                __attribute__((format(printf, 6, 7)));

#ifdef __cplusplus
} /* End of extern "C" */
#endif

#endif /* End of _LIB_STRATIS_PRIVATE_H_ */
