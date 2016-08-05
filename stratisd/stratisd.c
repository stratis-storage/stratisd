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

#include "../lib/stratis-common.h"
#include "libstratis.h"

#define PORT 8888  // TODO change this

int answer_to_connection(void *cls, struct MHD_Connection *connection,
        const char *url, const char *method, const char *version,
        const char *upload_data, size_t *upload_data_size, void **con_cls) {

	struct plugin *plugin = NULL;
	const char *answer = NULL;
	struct MHD_Response *response;
	int ret;

	answer = "<html><body>Response from stratisd</body></html>";

	response = MHD_create_response_from_buffer(strlen(answer), (void*) answer,
	        MHD_RESPMEM_PERSISTENT);
	ret = MHD_queue_response(connection, MHD_HTTP_OK, response);
	MHD_destroy_response(response);

	return ret;
}

int main(int argc, char **argv) {

	struct MHD_Daemon *daemon;

	daemon = MHD_start_daemon(MHD_USE_SELECT_INTERNALLY, PORT, NULL, NULL,
	        &answer_to_connection, NULL, MHD_OPTION_END);
	if (NULL == daemon)
		printf("Failed to start HTTP daemon");

	stratis_main_loop(NULL);

	MHD_stop_daemon(daemon);

	printf("exiting...\n");

}
