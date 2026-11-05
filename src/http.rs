//! GPUI's `HttpClient` backed by reqwest with an HTTP cache on disk, so `img()` can load
//! thumbnails and reuse them across launches. Adapted from gpui-kit's image guide.

use std::path::Path;

use futures::{AsyncReadExt as _, FutureExt as _, TryStreamExt as _, future::BoxFuture};
use gpui_kit::http_client::{
    AsyncBody, HttpClient, RedirectPolicy, Request, Response, Url, http::HeaderValue,
};
use http_cache_reqwest::{CACacheManager, Cache, CacheMode, HttpCache, HttpCacheOptions};
use reqwest::redirect;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};

pub struct CachedHttpClient {
    follow: ClientWithMiddleware,
    no_follow: ClientWithMiddleware,
    // reqwest needs a Tokio runtime; GPUI's executors are not one.
    tokio: tokio::runtime::Handle,
}

impl CachedHttpClient {
    pub fn new(cache_dir: &Path, tokio: tokio::runtime::Handle) -> anyhow::Result<Self> {
        let client = |policy, dir| -> anyhow::Result<_> {
            let client = reqwest::Client::builder().redirect(policy).build()?;
            Ok(ClientBuilder::new(client)
                .with(Cache(HttpCache {
                    mode: CacheMode::Default,
                    manager: CACacheManager {
                        path: cache_dir.join(dir),
                    },
                    options: HttpCacheOptions::default(),
                }))
                .build())
        };
        Ok(Self {
            follow: client(redirect::Policy::default(), "follow")?,
            no_follow: client(redirect::Policy::none(), "no-follow")?,
            tokio,
        })
    }
}

impl HttpClient for CachedHttpClient {
    fn user_agent(&self) -> Option<&HeaderValue> {
        None
    }

    fn proxy(&self) -> Option<&Url> {
        None
    }

    fn send(
        &self,
        req: Request<AsyncBody>,
    ) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
        let (head, mut body) = req.into_parts();
        let client = match head.extensions.get::<RedirectPolicy>() {
            Some(RedirectPolicy::NoFollow) => self.no_follow.clone(),
            _ => self.follow.clone(),
        };
        let tokio = self.tokio.clone();
        async move {
            let mut bytes = Vec::new();
            body.read_to_end(&mut bytes).await?;
            let request = client
                .request(head.method, head.uri.to_string())
                .headers(head.headers)
                .body(bytes);
            let response = tokio.spawn(request.send()).await??;

            let mut builder = Response::builder()
                .status(response.status())
                .version(response.version());
            *builder.headers_mut().unwrap() = response.headers().clone();
            let body = response
                .bytes_stream()
                .map_err(std::io::Error::other)
                .into_async_read();
            Ok(builder.body(AsyncBody::from_reader(body))?)
        }
        .boxed()
    }
}
