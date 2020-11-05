// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::{collections::HashMap, error::Error, future::Future, io, path::Path, pin::Pin};

use futures_util::FutureExt;
use jsonrpsee::{
    common::{self, ErrorCode, Failure, Id, Output, Request, Response, Version},
    transport::{TransportClient, TransportServer, TransportServerEvent},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
};

pub struct UdsTransportClient {
    socket: UnixStream,
}

impl UdsTransportClient {
    pub async fn connect<P>(path: P) -> io::Result<UdsTransportClient>
    where
        P: AsRef<Path>,
    {
        Ok(UdsTransportClient {
            socket: UnixStream::connect(path).await?,
        })
    }
}

type SendRequest<'a> = Pin<Box<dyn Future<Output = Result<(), Box<dyn Error + Send>>> + Send + 'a>>;
type NextResponse<'a> =
    Pin<Box<dyn Future<Output = Result<Response, Box<dyn Error + Send>>> + Send + 'a>>;

impl TransportClient for UdsTransportClient {
    type Error = Box<dyn Error + Send>;

    fn send_request(&mut self, request: Request) -> SendRequest {
        Box::pin(async move {
            let bytes =
                serde_json::to_vec(&request).map_err(|e| Box::new(e) as Box<dyn Error + Send>)?;
            self.socket
                .write(bytes.as_slice())
                .map(|res| {
                    res.map(|_| ())
                        .map_err(|e| Box::new(e) as Box<dyn Error + Send>)
                })
                .await
        })
    }

    fn next_response(&mut self) -> NextResponse {
        Box::pin(async move {
            let mut bytes = vec![0; 4096];
            let bytes_read = self
                .socket
                .read(bytes.as_mut_slice())
                .await
                .map_err(|e| Box::new(e) as Box<dyn Error + Send>)?;
            let response: Response = serde_json::from_slice(&bytes[0..bytes_read])
                .map_err(|e| Box::new(e) as Box<dyn Error + Send>)?;
            Ok(response)
        })
    }
}

pub struct UdsTransportServer {
    listener: UnixListener,
    stream_map: HashMap<u64, UnixStream>,
}

impl UdsTransportServer {
    pub fn bind<P>(path: P) -> io::Result<UdsTransportServer>
    where
        P: AsRef<Path>,
    {
        Ok(UdsTransportServer {
            listener: UnixListener::bind(path)?,
            stream_map: HashMap::new(),
        })
    }
}

impl TransportServer for UdsTransportServer {
    type RequestId = u64;

    fn next_request<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = TransportServerEvent<u64>> + Send + 'a>> {
        Box::pin(async move {
            let rand: u64 = rand::random();
            let mut stream = match self.listener.accept().await {
                Ok((stream, _)) => stream,
                Err(_) => return TransportServerEvent::Closed(rand),
            };

            let mut vec = vec![0; 4096];
            let bytes_read = match stream.read(&mut vec).await {
                Ok(br) => br,
                Err(_) => return TransportServerEvent::Closed(rand),
            };

            let request: Request = match serde_json::from_slice(&vec[0..bytes_read]) {
                Ok(req) => req,
                Err(e) => {
                    let response = Response::Single(Output::Failure(Failure {
                        jsonrpc: Version::V2,
                        error: common::Error {
                            code: ErrorCode::InvalidRequest,
                            message: format!(
                                "Could not deserialize the provided JSON bytes {:?}: {}",
                                vec, e,
                            ),
                            data: None,
                        },
                        id: Id::Num(rand),
                    }));
                    let vec = match serde_json::to_vec(&response) {
                        Ok(v) => v,
                        Err(_) => return TransportServerEvent::Closed(rand),
                    };
                    let _ = stream.write_all(vec.as_slice());
                    return TransportServerEvent::Closed(rand);
                }
            };

            self.stream_map.insert(rand, stream);

            TransportServerEvent::Request { id: rand, request }
        })
    }

    fn send<'a>(
        &'a mut self,
        _: &u64,
        _: &'a Response,
    ) -> Pin<Box<dyn Future<Output = Result<(), ()>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }

    fn supports_resuming(&self, _: &u64) -> Result<bool, ()> {
        Ok(true)
    }

    fn finish<'a>(
        &'a mut self,
        id: &'a u64,
        response: Option<&'a Response>,
    ) -> Pin<Box<dyn Future<Output = Result<(), ()>> + Send + 'a>> {
        Box::pin(async move {
            let mut stream = self.stream_map.remove(id).ok_or_else(|| ())?;
            match response {
                Some(resp) => {
                    let vec = match serde_json::to_vec(resp) {
                        Ok(v) => v,
                        Err(_) => return Err(()),
                    };
                    stream.write_all(vec.as_slice()).await.map_err(|_| ())?;
                    Ok(())
                }
                None => Ok(()),
            }
        })
    }
}
