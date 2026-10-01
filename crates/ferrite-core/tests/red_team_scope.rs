//! Red-team table for origin parsing and scope admission (ADR-014): every way
//! a URL can be spelled so that a scope for one site admits another, or fails
//! to admit its own. "Admits" here is what the comparator relies on to decide
//! whether an action stayed inside what the task needed.

use ferrite_core::Origin;
use ferrite_core::scope::{DomainSuffix, OriginScope, Specificity};

fn origin(url: &str) -> Origin {
    Origin::parse(url).unwrap_or_else(|e| panic!("{url}: {e}"))
}

fn exact(url: &str) -> OriginScope {
    OriginScope::exact([origin(url)]).expect("non-empty")
}

fn suffix(s: &str) -> OriginScope {
    OriginScope::domain_suffix([DomainSuffix::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))])
        .expect("non-empty")
}

/// URLs an attacker can write so that they *look* like `https://bank.example`
/// to a person or to a naive string check. None may be admitted by a scope for
/// the real site.
const LOOKALIKES: &[&str] = &[
    "https://bank.example.evil.com/",
    "https://bank.example@evil.com/",
    "https://bank.example:pw@evil.com/",
    "https://evil.com\\@bank.example/",
    "https://evil.com/\\@bank.example",
    "https://evil.com#@bank.example",
    "https://evil.com?@bank.example",
    "https://evil.com/?https://bank.example",
    "https://evil.com/https://bank.example/",
    "https://bank.example.evil.com:443/",
    "https://bank-example.com/",
    "https://bank.example-login.com/",
    "https://bankexample.com/",
    "https://xn--bnk-example-x3a.com/",
    "https://bаnk.example/", // Cyrillic а
    "https://bank.еxample/", // Cyrillic е
    "https://bank.example%2eevil.com/",
    "https://bank.example%00.evil.com/",
    "https://evil.com/bank.example/",
    "http://bank.example.evil.com/",
    "https://www.bank.example.evil.com/",
    "https://evil.com:443@bank.example.evil.com/",
    "https://BANK.EXAMPLE.EVIL.COM/",
];

#[test]
fn no_lookalike_is_admitted_by_an_exact_or_suffix_scope_for_the_real_site() {
    let exact_scope = exact("https://bank.example");
    let suffix_scope = suffix("bank.example");
    for url in LOOKALIKES {
        let Ok(o) = Origin::parse(url) else { continue };
        assert_eq!(
            exact_scope.admits(&o),
            None,
            "exact scope admitted {url} -> {o}"
        );
        assert_eq!(
            suffix_scope.admits(&o),
            None,
            "suffix scope admitted {url} -> {o}"
        );
    }
}

#[test]
fn spellings_of_the_same_site_are_admitted() {
    let scope = exact("https://bank.example");
    for url in [
        "https://bank.example",
        "https://BANK.Example/",
        "https://bank.example:443/",
        "https://bank.example/path?q=1#frag",
        "https://user:pw@bank.example/",
        "HTTPS://bank.example",
        "https:bank.example",
        "https://bank.example\t/",
        "https://ban\tk.example/",
        "https://bank.example\u{200b}/",
    ] {
        assert_eq!(
            scope.admits(&origin(url)),
            Some(Specificity::Exact),
            "{url}"
        );
    }
}

#[test]
fn exact_scopes_distinguish_scheme_and_port() {
    let scope = exact("https://bank.example");
    for url in [
        "http://bank.example",
        "https://bank.example:8443",
        "https://bank.example:80",
        "http://bank.example:443",
    ] {
        assert_eq!(scope.admits(&origin(url)), None, "{url}");
    }
    // The default port is the same origin however it is written.
    assert_eq!(
        origin("https://bank.example:443"),
        origin("https://bank.example")
    );
    assert_eq!(
        origin("http://bank.example:80"),
        origin("http://bank.example")
    );
}

#[test]
fn a_trailing_dot_fails_closed_not_open() {
    // `bank.example.` is the same DNS name, but it is a different origin string
    // and is not admitted: a false alarm at worst, never a bypass.
    assert_eq!(
        exact("https://bank.example").admits(&origin("https://bank.example.")),
        None
    );
    assert_eq!(
        suffix("bank.example").admits(&origin("https://bank.example.")),
        None
    );
}

#[test]
fn domain_suffix_scopes_admit_subdomains_on_label_boundaries_only() {
    let scope = suffix("bank.example");
    for ok in [
        "https://bank.example",
        "https://www.bank.example",
        "https://a.b.c.bank.example",
        "https://WWW.BANK.EXAMPLE",
    ] {
        assert_eq!(
            scope.admits(&origin(ok)),
            Some(Specificity::DomainSuffix),
            "{ok}"
        );
    }
    for bad in [
        "https://notbank.example",
        "https://bank.example.com",
        "https://xbank.example",
        "https://bank.examples",
        "https://bank-example",
    ] {
        assert_eq!(scope.admits(&origin(bad)), None, "{bad}");
    }
}

#[test]
fn domain_suffix_scopes_ignore_scheme_and_port_by_design() {
    // Pinned so that a change is deliberate: a suffix scope is "a bounded
    // family of hosts" (ADR-004); use an exact scope when scheme or port matter.
    let scope = suffix("bank.example");
    assert_eq!(
        scope.admits(&origin("http://bank.example")),
        Some(Specificity::DomainSuffix)
    );
    assert_eq!(
        scope.admits(&origin("https://bank.example:8443")),
        Some(Specificity::DomainSuffix)
    );
}

#[test]
fn ip_addresses_normalize_so_their_spellings_cannot_dodge_a_scope() {
    let loopback = exact("http://127.0.0.1");
    for url in [
        "http://127.0.0.1",
        "http://2130706433/",
        "http://0x7f.1/",
        "http://127.1/",
        "http://0177.0.0.1/",
        "http://0x7f000001/",
    ] {
        assert_eq!(
            loopback.admits(&origin(url)),
            Some(Specificity::Exact),
            "{url}"
        );
    }
    // A scope for localhost is not a scope for 127.0.0.1 and vice versa.
    assert_eq!(
        exact("http://localhost").admits(&origin("http://127.0.0.1")),
        None
    );
    assert_eq!(loopback.admits(&origin("http://[::1]")), None);
    assert_eq!(loopback.admits(&origin("http://[::ffff:127.0.0.1]")), None);
}

#[test]
fn only_http_and_https_have_an_origin_a_scope_can_admit() {
    for url in [
        "javascript:alert(1)",
        "data:text/html,<script>1</script>",
        "blob:https://bank.example/0e1c",
        "file:///etc/passwd",
        "about:blank",
        "about:srcdoc",
        "ftp://bank.example/",
        "ws://bank.example/",
        "wss://bank.example/",
        "view-source:https://bank.example",
        "chrome://settings",
        "mailto:a@bank.example",
        "tel:+15551234567",
        "intent://scan/#Intent;scheme=zxing;end",
        "JaVaScRiPt:alert(1)",
        " javascript:alert(1)",
        "\tjavascript:alert(1)",
    ] {
        assert!(Origin::parse(url).is_err(), "{url} parsed as an origin");
    }
}

#[test]
fn unparsable_and_relative_inputs_are_errors_not_origins() {
    for url in [
        "",
        " ",
        "bank.example",
        "//bank.example",
        "/path",
        "?q=1",
        "#frag",
        "https://",
        "https:///",
        "https://:443",
        "https://bank.example:99999",
        "https://bank.example:-1",
        "https://[::1",
        "https://bank .example/",
        "https://ba<nk.example/",
        "https://bank.example\u{0}/",
        "http://[fe80::1%25eth0]/",
    ] {
        assert!(Origin::parse(url).is_err(), "{url:?} parsed as an origin");
    }
}

#[test]
fn public_suffixes_are_refused_as_domain_suffix_scopes() {
    for bad in [
        "com",
        "uk",
        "co.uk",
        "org.uk",
        "com.au",
        "github.io",
        "gitlab.io",
        "blogspot.com",
        "herokuapp.com",
        "vercel.app",
        "pages.dev",
        "netlify.app",
        "appspot.com",
        "cloudfront.net",
        "s3.amazonaws.com",
        "myshopify.com",
        "localhost",
        "test",
        "a",
        "xn--p1ai",
        "com.",
        "*.com",
        ".co.uk",
        "1.2.3.4",
        "10.0.0",
        "0",
    ] {
        assert!(
            DomainSuffix::parse(bad).is_err(),
            "{bad:?} was accepted as a domain suffix and would admit every site beneath it"
        );
    }
}

#[test]
fn registrable_domains_are_accepted_as_domain_suffix_scopes() {
    for good in [
        "example.com",
        "example.co.uk",
        "foo.github.io",
        "me.vercel.app",
        "shop.myshopify.com",
        "bank.example",
        "sub.example.com",
        "xn--bcher-kva.example",
        "EXAMPLE.COM",
        "*.example.com",
        ".example.com",
        "my-site.test",
    ] {
        assert!(DomainSuffix::parse(good).is_ok(), "{good:?} was rejected");
    }
}

#[test]
fn a_suffix_scope_for_a_shared_host_admits_only_that_tenant() {
    let scope = suffix("alice.github.io");
    assert_eq!(
        scope.admits(&origin("https://alice.github.io")),
        Some(Specificity::DomainSuffix)
    );
    assert_eq!(scope.admits(&origin("https://mallory.github.io")), None);
    assert_eq!(scope.admits(&origin("https://github.io")), None);
}

#[test]
fn json_scopes_take_the_same_validation_route() {
    for bad in [
        r#"{"domain_suffix":["com"]}"#,
        r#"{"domain_suffix":["example.com","co.uk"]}"#,
        r#"{"domain_suffix":["github.io"]}"#,
    ] {
        assert!(
            serde_json::from_str::<OriginScope>(bad).is_err(),
            "{bad} deserialized"
        );
    }
    assert!(serde_json::from_str::<OriginScope>(r#"{"domain_suffix":["example.com"]}"#).is_ok());
}

#[test]
fn an_empty_or_blank_scope_admits_nothing_and_task_open_needs_a_reason() {
    assert!(OriginScope::exact([]).is_err());
    assert!(OriginScope::domain_suffix([]).is_err());
    assert!(OriginScope::task_open("").is_err());
    assert!(OriginScope::task_open("   \n\t").is_err());
    assert!(serde_json::from_str::<OriginScope>(r#"{"task_open":{"rationale":""}}"#).is_err());
    assert!(serde_json::from_str::<OriginScope>(r#"{"exact":[]}"#).is_err());
}

#[test]
fn task_open_is_the_weakest_tier_and_admits_everything_that_parses() {
    let open = OriginScope::task_open("user asked to browse freely").expect("rationale given");
    assert_eq!(
        open.admits(&origin("https://anything.example")),
        Some(Specificity::TaskOpen)
    );
    assert!(
        Specificity::TaskOpen < Specificity::DomainSuffix
            && Specificity::DomainSuffix < Specificity::Exact
    );
}

#[test]
fn hostile_strings_never_panic() {
    let long_host = format!("https://{}.example/", "a".repeat(10_000));
    let many_labels = format!("https://{}example/", "a.".repeat(5_000));
    for url in [
        long_host.as_str(),
        many_labels.as_str(),
        "https://\u{202e}bank.example/",
        "https://bank.example/\u{feff}",
        "https://[::]:0/",
        "http://0/",
        "http://256.256.256.256/",
        "https://%/",
        "https://%zz.example/",
        "https://xn--/",
        "https://xn--a-ecp.ru/",
        "\u{0}",
    ] {
        let _ = Origin::parse(url);
    }
    for s in [
        "\u{0}",
        "\u{202e}.com",
        &"a.".repeat(5_000),
        &"é".repeat(1_000),
        "xn--",
        "..",
        "-",
        "a..b",
    ] {
        let _ = DomainSuffix::parse(s);
    }
}
