use std::fmt::Write;

pub fn service_generator() -> Box<ServiceGenerator> {
    Box::new(ServiceGenerator {})
}

pub struct ServiceGenerator;

impl prost_build::ServiceGenerator for ServiceGenerator {
    fn generate(&mut self, service: prost_build::Service, buf: &mut String) {
        let service_name = service.name.replace("Api", "API");
        let service_fqn = format!("{}.{}", service.package, service_name);
        writeln!(buf).unwrap();
        writeln!(buf, "#[async_trait::async_trait]").unwrap();
        writeln!(buf, "pub trait {} {{", service_name).unwrap();
        for m in &service.methods {
            writeln!(
                buf,
                "    async fn {}(&self, req: {}) -> Result<{}, twirp::TwirpErrorResponse>;",
                m.name, m.input_type, m.output_type,
            )
            .unwrap();
        }
        writeln!(buf, "}}").unwrap();

        // add_service
        writeln!(
            buf,
            r#"pub fn add_service<T>(router: &mut twirp::Router, api: std::sync::Arc<T>)
where
    T: {} + Send + Sync + 'static,
{{"#,
            service_name
        )
        .unwrap();
        for m in &service.methods {
            writeln!(
                buf,
                r#"    {{
        #[allow(clippy::redundant_clone)]
        let api = api.clone();
        router.add_method(
            "/twirp/{}/{}",
            move |req| {{
                let api = api.clone();
                async move {{ api.{}(req).await }}
            }},
        );
    }}"#,
                service_fqn, m.proto_name, m.name
            )
            .unwrap();
        }
        writeln!(buf, "}}").unwrap();
    }
}