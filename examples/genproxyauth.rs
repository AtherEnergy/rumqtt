extern crate jsonwebtoken;
#[macro_use]
extern crate serde_derive;
extern crate chrono;

use jsonwebtoken::{encode, Algorithm, Header};
use chrono::Utc;

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    iat: i64,
    exp: i64,
    aud: String,
}

fn main() {
    let key = include_bytes!("gcloudfiles/rsa_private.der");
    let time = Utc::now();
    let jwt_header = Header::new(Algorithm::RS256);
    let iat = time.timestamp();
    let exp = time.checked_add_signed(chrono::Duration::minutes(5))
        .expect("Unable to create expiry")
        .timestamp();

    let claims = Claims { iat, exp, aud: "hello world".to_string()};
    let jwt = encode(&jwt_header, &claims, key).unwrap();
    // let jwt64 = base64::encode(jwt.as_bytes());
    println!("{}", jwt);
}

