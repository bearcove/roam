//! Generate postcard primitive/varint golden vectors used by TypeScript tests.

use std::{fs, path::PathBuf};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("test-fixtures")
        .join("golden-vectors")
}

fn write_fixture(path: &str, bytes: &[u8]) {
    let out_path = fixture_root().join(path);
    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent).expect("failed to create fixture directory");
    }
    fs::write(&out_path, bytes).expect("failed to write fixture");
    eprintln!("wrote {} ({} bytes)", out_path.display(), bytes.len());
}

fn main() {
    macro_rules! write_value {
        ($path:literal, $value:expr) => {{
            let bytes = facet_postcard::to_vec(&$value).expect("serialize fixture");
            write_fixture($path, &bytes);
        }};
    }

    // ---------------------------------------------------------------------
    // Varint
    // ---------------------------------------------------------------------
    write_value!("varint/u64_0.bin", 0u64);
    write_value!("varint/u64_1.bin", 1u64);
    write_value!("varint/u64_127.bin", 127u64);
    write_value!("varint/u64_128.bin", 128u64);
    write_value!("varint/u64_255.bin", 255u64);
    write_value!("varint/u64_256.bin", 256u64);
    write_value!("varint/u64_16383.bin", 16383u64);
    write_value!("varint/u64_16384.bin", 16384u64);
    write_value!("varint/u64_65535.bin", 65535u64);
    write_value!("varint/u64_65536.bin", 65536u64);
    write_value!("varint/u64_1048576.bin", 1_048_576u64);

    // ---------------------------------------------------------------------
    // Primitives
    // ---------------------------------------------------------------------
    write_value!("primitives/bool_false.bin", false);
    write_value!("primitives/bool_true.bin", true);

    write_value!("primitives/u8_0.bin", 0u8);
    write_value!("primitives/u8_127.bin", 127u8);
    write_value!("primitives/u8_255.bin", 255u8);

    write_value!("primitives/i8_0.bin", 0i8);
    write_value!("primitives/i8_neg1.bin", -1i8);
    write_value!("primitives/i8_127.bin", 127i8);
    write_value!("primitives/i8_neg128.bin", -128i8);

    write_value!("primitives/u16_0.bin", 0u16);
    write_value!("primitives/u16_127.bin", 127u16);
    write_value!("primitives/u16_128.bin", 128u16);
    write_value!("primitives/u16_255.bin", 255u16);
    write_value!("primitives/u16_256.bin", 256u16);
    write_value!("primitives/u16_max.bin", u16::MAX);

    write_value!("primitives/i16_0.bin", 0i16);
    write_value!("primitives/i16_1.bin", 1i16);
    write_value!("primitives/i16_neg1.bin", -1i16);
    write_value!("primitives/i16_127.bin", 127i16);
    write_value!("primitives/i16_128.bin", 128i16);
    write_value!("primitives/i16_max.bin", i16::MAX);
    write_value!("primitives/i16_min.bin", i16::MIN);

    write_value!("primitives/u32_0.bin", 0u32);
    write_value!("primitives/u32_1.bin", 1u32);
    write_value!("primitives/u32_127.bin", 127u32);
    write_value!("primitives/u32_128.bin", 128u32);
    write_value!("primitives/u32_255.bin", 255u32);
    write_value!("primitives/u32_256.bin", 256u32);
    write_value!("primitives/u32_max.bin", u32::MAX);

    write_value!("primitives/i32_0.bin", 0i32);
    write_value!("primitives/i32_1.bin", 1i32);
    write_value!("primitives/i32_neg1.bin", -1i32);
    write_value!("primitives/i32_127.bin", 127i32);
    write_value!("primitives/i32_128.bin", 128i32);
    write_value!("primitives/i32_neg128.bin", -128i32);
    write_value!("primitives/i32_max.bin", i32::MAX);
    write_value!("primitives/i32_min.bin", i32::MIN);

    write_value!("primitives/u64_0.bin", 0u64);
    write_value!("primitives/u64_1.bin", 1u64);
    write_value!("primitives/u64_127.bin", 127u64);
    write_value!("primitives/u64_128.bin", 128u64);
    write_value!("primitives/u64_max.bin", u64::MAX);

    write_value!("primitives/i64_0.bin", 0i64);
    write_value!("primitives/i64_1.bin", 1i64);
    write_value!("primitives/i64_neg1.bin", -1i64);
    write_value!("primitives/i64_15.bin", 15i64);
    write_value!("primitives/i64_42.bin", 42i64);
    write_value!("primitives/i64_max.bin", i64::MAX);
    write_value!("primitives/i64_min.bin", i64::MIN);

    write_value!("primitives/f32_0.bin", 0.0f32);
    write_value!("primitives/f32_1.bin", 1.0f32);
    write_value!("primitives/f32_neg1.bin", -1.0f32);
    write_value!("primitives/f32_1_5.bin", 1.5f32);
    write_value!("primitives/f32_0_25.bin", 0.25f32);

    write_value!("primitives/f64_0.bin", 0.0f64);
    write_value!("primitives/f64_1.bin", 1.0f64);
    write_value!("primitives/f64_neg1.bin", -1.0f64);
    write_value!("primitives/f64_1_5.bin", 1.5f64);
    write_value!("primitives/f64_0_25.bin", 0.25f64);

    write_value!("primitives/string_empty.bin", String::new());
    write_value!("primitives/string_hello.bin", "hello world".to_string());
    write_value!("primitives/string_unicode.bin", "héllo 世界 🦀".to_string());

    write_value!("primitives/bytes_empty.bin", Vec::<u8>::new());
    write_value!(
        "primitives/bytes_deadbeef.bin",
        vec![0xDEu8, 0xAD, 0xBE, 0xEF]
    );

    write_value!("primitives/option_none_u32.bin", Option::<u32>::None);
    write_value!("primitives/option_some_u32_42.bin", Some(42u32));
    write_value!("primitives/option_none_string.bin", Option::<String>::None);
    write_value!(
        "primitives/option_some_string.bin",
        Some("hello".to_string())
    );

    write_value!("primitives/vec_empty_u32.bin", Vec::<u32>::new());
    write_value!("primitives/vec_u32_1_2_3.bin", vec![1u32, 2, 3]);
    write_value!("primitives/vec_i32_neg1_0_1.bin", vec![-1i32, 0, 1]);
    write_value!(
        "primitives/vec_string.bin",
        vec!["a".to_string(), "b".to_string()]
    );
}
