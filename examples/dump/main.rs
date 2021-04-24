use gcemeta::Client;

macro_rules! call {
	  ($c:expr, $name:ident($($tt:tt)*)) => {
		    println!(concat!(stringify!($name), " = {:?}"), $c.$name($($tt)*).await);
	  };
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let c = Client::new();

    call!(c, on_gce());
    call!(c, project_id());
    call!(c, numeric_project_id());
    call!(c, internal_ip());
    call!(c, external_ip());
    call!(c, email(None));
    call!(c, hostname());
    call!(c, instance_tags());
    call!(c, instance_id());
    call!(c, instance_name());
    call!(c, zone());
    call!(c, instance_attrs());
    call!(c, project_attrs());
    call!(c, instance_attr("attr"));
    call!(c, project_attr("attr"));
    call!(c, scopes(None));
}
