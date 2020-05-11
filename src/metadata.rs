use attohttpc::{body::Empty, PreparedRequest, RequestBuilder, StatusCode};
use lazy_static::lazy_static;

use std::{
    env,
    net::{SocketAddr, ToSocketAddrs},
    sync::mpsc::{self, Sender},
    thread,
    time::Duration,
};

use crate::{ErrorKind, Result};

lazy_static! {
    static ref ON_GCE: bool = test_on_gce();
}

// Environment variable specifying the GCE metadata hostname.
// If empty, the default value of `METADATA_IP` is used instead.
const METADATA_HOST_VAR: &str = "GCE_METADATA_HOST";

// Documented metadata server IP address.
const METADATA_IP: &str = "169.254.169.254";

// `github.com/{user}/{package_name} v{package_version}`
const USER_AGENT: &str = concat!(
    "github.com/mechiru/",
    env!("CARGO_PKG_NAME"),
    " v",
    env!("CARGO_PKG_VERSION")
);

fn httpc_get<T>(url: T) -> RequestBuilder
where
    T: AsRef<str>,
{
    attohttpc::get(url).header_append(attohttpc::header::USER_AGENT, USER_AGENT)
}

/// Report whether this process is running on Google Compute Engine.
pub fn on_gce() -> bool {
    *ON_GCE
}

fn test_on_gce() -> bool {
    if env::var(METADATA_HOST_VAR).is_ok() {
        return true;
    }

    let (tx, rx) = mpsc::channel();

    has_meta_header(
        tx.clone(),
        httpc_get(format!("http://{}", METADATA_IP)).prepare(),
    );

    has_target_ip(tx, "metadata.google.internal", METADATA_IP);

    rx.recv().unwrap_or(false)
        || running_on_gce() && rx.recv_timeout(Duration::from_secs(5)).unwrap_or(false)
}

fn has_meta_header(tx: Sender<bool>, mut req: PreparedRequest<Empty>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let ret = req
            .send()
            .map(|resp| match resp.headers().get("Metadata-Flavor") {
                Some(v) => v == "Google",
                None => false,
            })
            .unwrap_or(false);
        let _ = tx.send(ret);
    })
}

fn has_target_ip(tx: Sender<bool>, host: &'static str, ip: &'static str) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        const DUMMY_PORT: u16 = 0;
        let addr: SocketAddr = format!("{}:{}", ip, DUMMY_PORT).parse().unwrap();
        let ret = (host, DUMMY_PORT)
            .to_socket_addrs()
            .map(|mut xs| xs.any(|x| x == addr))
            .unwrap_or(false);
        let _ = tx.send(ret);
    })
}

fn running_on_gce() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/sys/class/dmi/id/product_name")
            .map(|c| match c.trim() {
                "Google" | "Google Compute Engine" => true,
                _ => false,
            })
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

const UNINITIALIZED: u8 = 0;
const INITIALIZING: u8 = 1;
const INITIALIZED: u8 = 2;

macro_rules! impl_cached_meta_fn {
    () => {};
    ($(#[$attr:meta])* $mod:ident::$fn:ident => $ty:ty, $suffix:expr, $map:path, $($t:tt)*) => {
        mod $mod {
            use std::sync::atomic::{AtomicU8, Ordering};
            use crate::metadata::{get_meta, INITIALIZED, INITIALIZING, UNINITIALIZED};

            // TODO: MaybeUninit
            static mut CACHE: Option<$ty> = None;
            static STATE: AtomicU8 = AtomicU8::new(UNINITIALIZED);

            $(#[$attr])*
            pub fn $fn() -> crate::Result<$ty> {
                match STATE.compare_and_swap(UNINITIALIZED, INITIALIZING, Ordering::SeqCst) {
                    UNINITIALIZED => match get_meta($suffix) {
                        Ok(v) => {
                            unsafe { CACHE = Some($map(v)) }
                            STATE.store(INITIALIZED, Ordering::SeqCst);
                            Ok(unsafe { CACHE.clone() }.unwrap())
                        }
                        Err(why) => {
                            STATE.store(UNINITIALIZED, Ordering::SeqCst);
                            Err(why.into())
                        }
                    },
                    INITIALIZING => loop {
                        match STATE.load(Ordering::SeqCst) {
                            INITIALIZING => continue,
                            _ => break unsafe { CACHE.clone() }
                                           .ok_or(crate::ErrorKind::Uninitialized.into()),
                        }
                    },
                    _ => Ok(unsafe { CACHE.clone() }.unwrap()),
                }
            }
        }

        pub use $mod::$fn;

        impl_cached_meta_fn!($($t)*);
    };
}

fn get_meta(suffix: &str) -> Result<String> {
    let host = env::var(METADATA_HOST_VAR).unwrap_or_else(|_| METADATA_IP.into());
    let resp = httpc_get(format!("http://{}/computeMetadata/v1/{}", host, suffix))
        .header_append("Metadata-Flavor", "Google")
        .connect_timeout(Duration::from_secs(2))
        .send()?;

    match resp.status() {
        StatusCode::OK => Ok(resp.text()?),
        code => Err(code.into()),
    }
}

/// Get value from the metadata service.
///
/// The suffix is appended to `http://${GCE_METADATA_HOST}/computeMetadata/v1/`.
/// If the `GCE_METADATA_HOST` environment variable is not defined, a default of
/// `169.254.169.254` will be used instead.
pub fn get(suffix: &str) -> Result<Option<String>> {
    get_meta(suffix).map(Some).or_else(|e| match e.kind() {
        ErrorKind::HttpResponse(StatusCode::NOT_FOUND) => Ok(None),
        _ => Err(e),
    })
}

impl_cached_meta_fn! {
    /// Get the current instance's project ID string.
    project_id_impl::project_id => String, "project/project-id", crate::metadata::trim,

    /// Get the current instance's numeric project ID.
    numeric_project_id_impl::numeric_project_id => String, "project/numeric-project-id", crate::metadata::trim,

    /// Get the current VM's numeric instance ID.
    instance_id_impl::instance_id => String, "instance/id", crate::metadata::trim,
}

/// Get the instance's primary internal IP address.
pub fn internal_ip() -> Result<String> {
    get_meta("instance/network-interfaces/0/ip").map(trim)
}

/// Get the instance's primary external (public) IP address.
pub fn external_ip() -> Result<String> {
    get_meta("instance/network-interfaces/0/access-configs/0/external-ip").map(trim)
}

/// Get the instance's hostname.
///
/// This will be of the form `<instance_id>.c.<project_id>.internal`.
pub fn hostname() -> Result<String> {
    get_meta("instance/hostname").map(trim)
}

/// Get the list of user-defined instance tags, assigned when initially creating a GCE instance.
pub fn instance_tags() -> Result<Vec<String>> {
    get_meta("instance/tags").and_then(json_array)
}

/// Get the current VM's instance ID string.
pub fn instance_name() -> Result<String> {
    hostname().and_then(parse_instance_name)
}

/// Get the current VM's zone, such as `us-central1-b`.
pub fn zone() -> Result<String> {
    get_meta("instance/zone").map(trim).and_then(parse_zone)
}

/// Get the list of user-defined attributes, assigned when initially creating a GCE VM instance.
pub fn instance_attributes() -> Result<Vec<String>> {
    get_meta("instance/attributes/").map(lines)
}

/// Get the list of user-defined attributes applying to the project as a whole, not just this VM.
pub fn project_attributes() -> Result<Vec<String>> {
    get_meta("project/attributes/").map(lines)
}

/// Get the value of the provided VM instance attribute.
pub fn instance_attribute_value(attr: &str) -> Result<Option<String>> {
    get(&format!("instance/attributes/{}", attr))
}

/// Get the value of the provided project attribute.
pub fn project_attribute_value(attr: &str) -> Result<Option<String>> {
    get(&format!("project/attributes/{}", attr))
}

/// Get the service account scopes for the given account.
///
/// The account may be `None` or `Some("default")` to use the instance's main account.
pub fn scopes(service_account: Option<&str>) -> Result<Vec<String>> {
    let sa = service_account.unwrap_or("default");
    get_meta(&format!("instance/service-accounts/{}/scopes", sa)).map(lines)
}

fn trim<S: AsRef<str>>(s: S) -> String {
    s.as_ref().trim().to_owned()
}

fn lines<S: AsRef<str>>(s: S) -> Vec<String> {
    s.as_ref()
        .lines()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
}

fn json_array<S: AsRef<str>>(s: S) -> Result<Vec<String>> {
    serde_json::from_str(s.as_ref()).map_err(|_| ErrorKind::MetadataParse("json array").into())
}

fn parse_instance_name<S: AsRef<str>>(s: S) -> Result<String> {
    s.as_ref()
        .split('.')
        .next()
        .filter(|x| !x.is_empty())
        .map(Into::into)
        .ok_or_else(|| ErrorKind::MetadataParse("instance name").into())
}

fn parse_zone<S: AsRef<str>>(s: S) -> Result<String> {
    s.as_ref()
        .split('/')
        .last()
        .filter(|x| !x.is_empty())
        .map(Into::into)
        .ok_or_else(|| ErrorKind::MetadataParse("zone").into())
}

#[cfg(test)]
mod test {
    use super::*;
    use rouille::{router, Response};

    lazy_static! {
        static ref PORT: u16 = {
            let server = rouille::Server::new("localhost:0", |req| {
                assert_eq!(req.header("User-Agent").unwrap(), USER_AGENT);
                router!(req,
                    (GET) ["/meta/has-header"] => Response::text("").with_additional_header("Metadata-Flavor", "Google"),
                    (GET) ["/meta/no-header"] => Response::text("").without_header(""),
                    (GET) ["/computeMetadata/v1/project/project-id"] => {
                        assert_eq!(req.header("Metadata-Flavor").unwrap(), "Google");
                        Response::text("<PROJECT_ID>").without_header("")
                    },
                    _ => Response::empty_404()
                )
            })
            .unwrap();
            let port = server.server_addr().port();
            thread::spawn(|| server.run());
            port
        };
    }

    #[test]
    fn has_meta_header_test() {
        let port = *PORT;

        let req = httpc_get(format!("http://localhost:{}/meta/has-header", port)).prepare();

        let (tx, rx) = mpsc::channel();
        has_meta_header(tx.clone(), req);
        assert!(rx.recv().unwrap());

        let req = httpc_get(format!("http://localhost:{}/meta/no-header", port)).prepare();
        has_meta_header(tx.clone(), req);
        assert!(!rx.recv().unwrap());
    }

    #[test]
    fn has_target_ip_test() {
        let (tx, rx) = mpsc::channel();

        has_target_ip(tx.clone(), "localhost", "127.0.0.1");
        assert!(rx.recv().unwrap());

        has_target_ip(tx.clone(), "localhost", "0.0.0.0");
        assert!(!rx.recv().unwrap());
    }

    #[test]
    fn get_meta_test() {
        let port = *PORT;
        env::set_var(METADATA_HOST_VAR, format!("localhost:{}", port));
        assert_eq!(get_meta("project/project-id").unwrap(), "<PROJECT_ID>");
        env::remove_var(METADATA_HOST_VAR);
    }

    #[test]
    fn trim_test() {
        assert_eq!(trim(""), "");
        assert_eq!(trim(" \r\n	abc \r\n	"), "abc");
    }

    #[test]
    fn lines_test() {
        assert_eq!(lines(""), Vec::<String>::new());
        assert_eq!(
            lines(
                r#"
a
	b
"#
            ),
            vec!["a", "b"]
        );
    }

    #[test]
    fn parse_instance_name_test() {
        match parse_instance_name("abc.c.ef.internal") {
            Ok(v) => assert_eq!(v, "abc"),
            _ => unreachable!(),
        }

        match parse_instance_name("").map_err(|e| e.into_kind()) {
            Ok(v) => unreachable!("parse_instance_name_test::2 {:?}", v),
            Err(ErrorKind::MetadataParse("instance name")) => {}
            _ => unreachable!(),
        }
    }

    #[test]
    fn parse_zone_test() {
        match parse_zone("projects/123/zones/asia-northeast1-a") {
            Ok(v) => assert_eq!(v, "asia-northeast1-a"),
            _ => unreachable!("parse_zone_test::1"),
        }

        match parse_zone("").map_err(|e| e.into_kind()) {
            Ok(_) => unreachable!(),
            Err(ErrorKind::MetadataParse("zone")) => {}
            _ => unreachable!(),
        }
    }
}
