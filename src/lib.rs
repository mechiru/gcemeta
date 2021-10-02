//! This library provides access to [`GCE metadata service`][`metadata`].
//!
//! # Example
//! ```rust,no_run
//! use gcemeta::Client;
//!
//! async fn example() {
//!     let client = Client::new();
//!     println!("on gce = {:?}", client.on_gce().await);
//!     println!("project id = {:?}", client.project_id().await);
//! }
//! ```
//!
//! [`metadata`]: https://developers.google.com/compute/docs/metadata

use hyper::{
    body::{aggregate, HttpBody},
    client::connect::Connect,
    header::{HeaderName, HeaderValue, USER_AGENT},
    http::{
        response::Parts,
        uri::{PathAndQuery, Scheme},
    },
    Body, Request, StatusCode, Uri,
};
use tokio::sync::RwLock;
use tracing::trace;

use std::{env, error, fmt, future::Future, str::FromStr, sync::Arc, time::Duration};

// === macros ===

macro_rules! __path {
    ($($expr:expr)*) => {
        concat!("/computeMetadata/v1/", $($expr)*)
    };
    ($expr:expr, $($tt:tt)*) => {
        format!(__path!($expr), $($tt)*)
    };
}

macro_rules! path {
    ($($expr:expr)*) => {
        PathAndQuery::from_static(__path!($($expr)*))
    };
    ($expr:expr, $($tt:tt)*) => {
        PathAndQuery::from_str(&__path!($expr, $($tt)*))
    };
}

macro_rules! impl_cache_fn {
    ($(#[$attr:meta])* $name:ident, $path:expr, $trim:expr) => {
        $(#[$attr])*
        pub async fn $name(&self) -> crate::Result<String> {
            if let Some(value) = self.cache.$name.read().await.clone() {
                return Ok(value);
            }
            let mut lock = self.cache.$name.write().await;
            if let Some(value) = lock.clone() {
                Ok(value)
            } else {
                let value = self.get(path!($path), $trim).await?;
                *lock = Some(value.clone());
                Ok(value)
            }
        }
    };
}

// === error ===

/// Represents errors that can occur during handling metadata service.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    // internal
    #[error("http client error: {0}")]
    Http(#[from] hyper::Error),
    // user
    #[error("uri parse error: {0}")]
    Uri(#[from] hyper::http::uri::InvalidUri),
    // server
    #[error("response status code error: {0:?}")]
    StatusCode((Parts, Body)),
    #[error("response body encoding error: {0}")]
    Encoding(#[from] std::string::FromUtf8Error),
    #[error("response body deserialize error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Wrapper for the `Result` type with an [`Error`](Error).
pub type Result<T> = std::result::Result<T, Error>;

// === env ===

#[derive(Clone)]
struct Env {
    metadata_host: Option<Uri>,
}

impl Env {
    fn init() -> Self {
        Self {
            // https://github.com/googleapis/google-cloud-go/blob/c66290a95b8bf2298d5e7c84378cb6118cc0a348/compute/metadata/metadata.go#L46
            metadata_host: env::var("GCE_METADATA_HOST")
                .ok()
                .map(|s| Uri::from_str(&s).expect("`GCE_METADATA_HOST` is not valid URI")),
        }
    }
}

// === config ===

#[derive(Clone)]
struct Config {
    schema: Scheme,
    metadata_ip: Uri,
    user_agent: HeaderValue,
    flavor_name: HeaderName,
    flavor_value: HeaderValue,
    probe_timeout: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema: Scheme::HTTP,
            // https://github.com/googleapis/google-cloud-go/blob/c66290a95b8bf2298d5e7c84378cb6118cc0a348/compute/metadata/metadata.go#L39
            metadata_ip: Uri::from_static("169.254.169.254"),
            user_agent: HeaderValue::from_static(concat!(
                "github.com/mechiru/",
                env!("CARGO_PKG_NAME"),
                " v",
                env!("CARGO_PKG_VERSION")
            )),
            flavor_name: HeaderName::from_static("metadata-flavor"),
            flavor_value: HeaderValue::from_static("Google"),
            probe_timeout: Duration::from_secs(5),
        }
    }
}

// === cache ===

#[derive(Default)]
struct Cache {
    on_gce: RwLock<Option<bool>>,
    project_id: RwLock<Option<String>>,
    numeric_project_id: RwLock<Option<String>>,
    instance_id: RwLock<Option<String>>,
}

// === client ===

/// A Client to access metadata service.
pub struct Client<C, B = Body> {
    inner: hyper::Client<C, B>,
    env: Env,
    config: Config,
    cache: Arc<Cache>,
}

#[allow(clippy::new_ret_no_self)]
impl Client<(), Body> {
    /// Create a new Client with the default config.
    #[cfg(feature = "default")]
    pub fn new() -> Client<hyper::client::connect::HttpConnector, Body> {
        // https://github.com/googleapis/google-cloud-go/blob/c66290a95b8bf2298d5e7c84378cb6118cc0a348/compute/metadata/metadata.go#L64-L71
        let inner = {
            let keepalive = Duration::from_secs(30);
            let mut connector = hyper::client::HttpConnector::new();
            connector.set_connect_timeout(Some(Duration::from_secs(2)));
            connector.set_keepalive(Some(keepalive));
            hyper::Client::builder().pool_idle_timeout(keepalive).build(connector)
        };
        Client { inner, env: Env::init(), config: Default::default(), cache: Default::default() }
    }

    /// Create a new client using the passed http client.
    #[cfg(not(feature = "default"))]
    pub fn new_with<C, B>(client: hyper::Client<C, B>) -> Client<C, B> {
        Client {
            inner: client,
            env: Env::init(),
            config: Default::default(),
            cache: Default::default(),
        }
    }
}

impl<C, B> Client<C, B>
where
    C: Connect + Clone + Send + Sync + 'static,
    B: HttpBody + Default + Send + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn error::Error + Send + Sync>>,
{
    fn get_parts(
        &self,
        path_and_query: PathAndQuery,
    ) -> impl Future<Output = crate::Result<(Parts, Body)>> + Send + 'static {
        let host = self.env.metadata_host.clone();
        let mut parts = host.unwrap_or_else(|| self.config.metadata_ip.clone()).into_parts();
        parts.scheme = Some(self.config.schema.clone());
        parts.path_and_query = Some(path_and_query);
        let uri = Uri::from_parts(parts).unwrap();

        let req = Request::get(uri)
            .header(&self.config.flavor_name, &self.config.flavor_value)
            .header(USER_AGENT, &self.config.user_agent)
            .body(B::default())
            .unwrap();
        let fut = self.inner.request(req);
        async {
            let parts = fut.await?.into_parts();
            match parts.0.status {
                StatusCode::OK => Ok(parts),
                _ => Err(Error::StatusCode(parts)),
            }
        }
    }

    /// Get value from the metadata service.
    pub fn get(
        &self,
        path_and_query: PathAndQuery,
        trim: bool,
    ) -> impl Future<Output = crate::Result<String>> + Send + 'static {
        use bytes::BufMut as _;

        let fut = self.get_parts(path_and_query);
        async move {
            let (_, mut body) = fut.await?;
            let mut vec = Vec::new();
            while let Some(next) = body.data().await {
                let chunk = next?;
                vec.put(chunk);
            }
            let mut s = String::from_utf8(vec)?;
            if trim {
                let trimed = s.trim();
                if trimed.len() != s.len() {
                    s = trimed.to_owned();
                }
            }
            Ok(s)
        }
    }

    /// Get value from the metadata service.
    pub fn get_as<T>(
        &self,
        path_and_query: PathAndQuery,
    ) -> impl Future<Output = crate::Result<T>> + Send + 'static
    where
        T: serde::de::DeserializeOwned,
    {
        use bytes::Buf as _;

        let fut = self.get_parts(path_and_query);
        async {
            let (_, body) = fut.await?;
            Ok(serde_json::from_reader(aggregate(body).await?.reader())?)
        }
    }

    /// Report whether this process is running on Google Compute Engine.
    pub async fn on_gce(&self) -> crate::Result<bool> {
        if let Some(on) = *self.cache.on_gce.read().await {
            return Ok(on);
        }

        let mut on_gce = self.cache.on_gce.write().await;
        if let Some(on) = *on_gce {
            return Ok(on);
        }

        let present = self.env.metadata_host.is_some();
        trace!("check environment variable: {}", present);
        if present {
            *on_gce = Some(true);
            return Ok(true);
        }

        let meta = async {
            let mut parts = self.config.metadata_ip.clone().into_parts();
            parts.scheme = Some(self.config.schema.clone());
            parts.path_and_query = Some(PathAndQuery::from_static("/"));

            let req = Request::get(Uri::from_parts(parts).unwrap())
                .header(&self.config.flavor_name, &self.config.flavor_value)
                .header(USER_AGENT, &self.config.user_agent)
                .body(B::default())
                .unwrap();

            let on = self
                .inner
                .request(req)
                .await
                .map(|resp| {
                    resp.headers().get(&self.config.flavor_name) == Some(&self.config.flavor_value)
                })
                .unwrap_or(false);
            trace!("access to medatada service: {}", on);
            on
        };

        let name = tokio::task::spawn_blocking(|| {
            use std::net::ToSocketAddrs as _;
            let on = ("metadata.google.internal", 0)
                .to_socket_addrs()
                .map(|addrs| addrs.len() > 0)
                .unwrap_or(false);
            trace!("resolve hostname: {}", on);
            on
        });

        let on = tokio::select! {
            true = meta => true,
            Ok(true) = name => true,
            _ = tokio::time::sleep(self.config.probe_timeout) => {
                trace!("probe timeout exceeded");
                false
            },
        };

        *on_gce = Some(on);
        Ok(on)
    }

    impl_cache_fn!(
        /// Get the current instance's project ID string.
        project_id,
        "project/project-id",
        true
    );

    impl_cache_fn!(
        /// Get the current instance's numeric project ID.
        numeric_project_id,
        "project/numeric-project-id",
        true
    );

    /// Get the instance's primary internal IP address.
    pub async fn internal_ip(&self) -> crate::Result<String> {
        self.get(path!("instance/network-interfaces/0/ip"), true).await
    }

    /// Get the instance's primary external (public) IP address.
    pub async fn external_ip(&self) -> crate::Result<String> {
        self.get(path!("instance/network-interfaces/0/access-configs/0/external-ip"), true).await
    }

    /// Get service account's email.
    pub async fn email(&self, sa: Option<&str>) -> crate::Result<String> {
        let path = match sa {
            Some(sa) => path!("instance/service-accounts/{}/email", sa)?,
            _ => path!("instance/service-accounts/default/email"),
        };
        self.get(path, true).await
    }

    /// Get the instance's hostname.
    ///
    /// This will be of the form `<instance_id>.c.<project_id>.internal`.
    pub async fn hostname(&self) -> crate::Result<String> {
        self.get(path!("instance/hostname"), true).await
    }

    /// Get the list of user-defined instance tags, assigned when initially creating a GCE instance.
    pub async fn instance_tags(&self) -> crate::Result<Vec<String>> {
        self.get_as(path!("instance/tags")).await
    }

    impl_cache_fn!(
        /// Get the current VM's numeric instance ID.
        instance_id,
        "instance/id",
        true
    );

    /// Get the current VM's instance ID string.
    pub async fn instance_name(&self) -> crate::Result<String> {
        self.get(path!("instance/name"), true).await
    }

    /// Get the current VM's zone, such as `us-central1-b`.
    pub async fn zone(&self) -> crate::Result<String> {
        // zone is of the form "projects/<numeric_project_id>/zones/<zone_name>".
        let s = self.get(path!("instance/zone"), true).await?;
        Ok(s.split('/').last().unwrap_or("").to_owned())
    }

    /// Get the list of user-defined attributes, assigned when initially creating a GCE VM instance.
    pub async fn instance_attrs(&self) -> crate::Result<Vec<String>> {
        let s = self.get(path!("instance/attributes/"), false).await?;
        Ok(s.lines().map(ToOwned::to_owned).collect())
    }

    /// Get the list of user-defined attributes applying to the project as a whole, not just this VM.
    pub async fn project_attrs(&self) -> crate::Result<Vec<String>> {
        let s = self.get(path!("project/attributes/"), false).await?;
        Ok(s.lines().map(ToOwned::to_owned).collect())
    }

    /// Get the value of the provided VM instance attribute.
    pub async fn instance_attr(&self, attr: impl AsRef<str>) -> crate::Result<String> {
        self.get(path!("instance/attributes/{}", attr.as_ref())?, false).await
    }

    /// Get the value of the provided project attribute.
    pub async fn project_attr(&self, attr: impl AsRef<str>) -> crate::Result<String> {
        self.get(path!("project/attributes/{}", attr.as_ref())?, false).await
    }

    /// Get the service account scopes for the given account.
    pub async fn scopes(&self, sa: Option<&str>) -> crate::Result<Vec<String>> {
        let path = match sa {
            Some(sa) => path!("instance/service-accounts/{}/scopes", sa)?,
            _ => path!("instance/service-accounts/default/scopes"),
        };
        let s = self.get(path, true).await?;
        Ok(s.lines().map(ToOwned::to_owned).collect())
    }

    // TODO: subscribe
}

impl<C: Clone, B> Clone for Client<C, B> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            env: self.env.clone(),
            config: self.config.clone(),
            cache: self.cache.clone(),
        }
    }
}

impl<C, B> fmt::Debug for Client<C, B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client").finish()
    }
}
