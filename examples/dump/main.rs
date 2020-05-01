use gcemeta::*;

macro_rules! dump {
    () => {};
    ($name:ident($($args:tt)*), $($t:tt)*) => {
        println!(concat!(stringify!($name), " = {:?}"), $name($($args)*));
        dump!($($t)*)
    };
}

fn main() {
    dump! {
        on_gce(),
        get("project/project-id"),
        project_id(),
        numeric_project_id(),
        internal_ip(),
        external_ip(),
        hostname(),
        instance_tags(),
        instance_id(),
        instance_name(),
        zone(),
        instance_attributes(),
        project_attributes(),
        instance_attribute_value("attr"),
        project_attribute_value("attr"),
        scopes(None),
    }
}
