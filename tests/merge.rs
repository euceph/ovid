use std::path::PathBuf;
use std::process::Command;

fn ovid_bin() -> PathBuf {
    // cargo test builds the binary in the target directory
    let mut path = std::env::current_exe().unwrap();
    // tests/merge-<hash> -> deps dir -> debug dir
    path.pop();
    path.pop();
    path.push("ovid");
    path
}

fn tmp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ovid_test_{}", name));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// run ovid merge and return the output PDF path
fn run_merge(images: &[PathBuf], out_pdf: &PathBuf) {
    let mut cmd = Command::new(ovid_bin());
    cmd.arg("merge");
    for img in images {
        cmd.arg(img);
    }
    cmd.arg("-o").arg(out_pdf);
    cmd.arg("--quiet");
    let output = cmd.output().expect("failed to run ovid");
    if !output.status.success() {
        panic!(
            "ovid merge failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

/// get the XObject image stream dictionary for "Im0" on the first page
fn get_first_page_image_dict(
    doc: &lopdf::Document,
) -> &lopdf::Dictionary {
    let pages = doc.get_pages();
    let page_id = pages.values().next().expect("no pages");
    let page_dict = doc.get_dictionary(*page_id).unwrap();
    let resources_ref = page_dict.get(b"Resources").unwrap();
    let (_, resources_obj) = doc.dereference(resources_ref).unwrap();
    let resources = resources_obj.as_dict().unwrap();
    let xobjects_ref = resources.get(b"XObject").unwrap();
    let (_, xobjects_obj) = doc.dereference(xobjects_ref).unwrap();
    let xobjects = xobjects_obj.as_dict().unwrap();
    let im0_ref = xobjects.get(b"Im0").unwrap();
    let (_, im0_obj) = doc.dereference(im0_ref).unwrap();
    match im0_obj {
        lopdf::Object::Stream(stream) => &stream.dict,
        _ => panic!("Im0 is not a stream"),
    }
}

fn write_tiny_jpeg_rgb(path: &PathBuf) {
    let img = image::RgbImage::from_fn(4, 4, |x, y| {
        image::Rgb([(x * 60) as u8, (y * 60) as u8, 128])
    });
    img.save(path).unwrap();
}

fn write_tiny_png_rgb(path: &PathBuf) {
    let img = image::RgbImage::from_fn(4, 4, |x, y| {
        image::Rgb([(x * 60) as u8, (y * 60) as u8, 200])
    });
    img.save(path).unwrap();
}

fn write_tiny_png_gray(path: &PathBuf) {
    let img = image::GrayImage::from_fn(4, 4, |x, _y| image::Luma([(x * 60) as u8]));
    img.save(path).unwrap();
}

fn write_tiny_png_rgba(path: &PathBuf) {
    let img = image::RgbaImage::from_fn(4, 4, |x, y| {
        image::Rgba([(x * 60) as u8, (y * 60) as u8, 100, 200])
    });
    img.save(path).unwrap();
}

/// write a tiny palette PNG using the png crate directly
fn write_tiny_png_palette(path: &PathBuf) {
    use std::io::BufWriter;

    let file = std::fs::File::create(path).unwrap();
    let w = BufWriter::new(file);
    let mut encoder = png::Encoder::new(w, 4, 4);
    encoder.set_color(png::ColorType::Indexed);
    encoder.set_depth(png::BitDepth::Eight);
    // 4-entry palette: red, green, blue, white
    encoder.set_palette(vec![
        255, 0, 0, // red
        0, 255, 0, // green
        0, 0, 255, // blue
        255, 255, 255, // white
    ]);
    let mut writer = encoder.write_header().unwrap();
    // 4x4 pixels using palette indices 0-3
    let data: Vec<u8> = vec![
        0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3, 0, 1, 2, 3,
    ];
    writer.write_image_data(&data).unwrap();
}

#[test]
fn test_merge_jpeg_rgb() {
    let dir = tmp_dir("jpeg_rgb");
    let img = dir.join("test.jpg");
    let pdf = dir.join("out.pdf");
    write_tiny_jpeg_rgb(&img);
    run_merge(&[img], &pdf);

    let doc = lopdf::Document::load(&pdf).unwrap();
    assert_eq!(doc.get_pages().len(), 1);

    let dict = get_first_page_image_dict(&doc);
    let filter = dict.get(b"Filter").unwrap();
    assert_eq!(filter.as_name_str().unwrap(), "DCTDecode");
    let cs = dict.get(b"ColorSpace").unwrap();
    assert_eq!(cs.as_name_str().unwrap(), "DeviceRGB");
}

#[test]
fn test_merge_png_rgb() {
    let dir = tmp_dir("png_rgb");
    let img = dir.join("test.png");
    let pdf = dir.join("out.pdf");
    write_tiny_png_rgb(&img);
    run_merge(&[img], &pdf);

    let doc = lopdf::Document::load(&pdf).unwrap();
    assert_eq!(doc.get_pages().len(), 1);

    let dict = get_first_page_image_dict(&doc);
    let filter = dict.get(b"Filter").unwrap();
    assert_eq!(filter.as_name_str().unwrap(), "FlateDecode");
    let cs = dict.get(b"ColorSpace").unwrap();
    assert_eq!(cs.as_name_str().unwrap(), "DeviceRGB");
}

#[test]
fn test_merge_png_gray() {
    let dir = tmp_dir("png_gray");
    let img = dir.join("test.png");
    let pdf = dir.join("out.pdf");
    write_tiny_png_gray(&img);
    run_merge(&[img], &pdf);

    let doc = lopdf::Document::load(&pdf).unwrap();
    assert_eq!(doc.get_pages().len(), 1);

    let dict = get_first_page_image_dict(&doc);
    let cs = dict.get(b"ColorSpace").unwrap();
    assert_eq!(cs.as_name_str().unwrap(), "DeviceGray");
}

#[test]
fn test_merge_png_rgba() {
    let dir = tmp_dir("png_rgba");
    let img = dir.join("test.png");
    let pdf = dir.join("out.pdf");
    write_tiny_png_rgba(&img);
    run_merge(&[img], &pdf);

    let doc = lopdf::Document::load(&pdf).unwrap();
    assert_eq!(doc.get_pages().len(), 1);

    let dict = get_first_page_image_dict(&doc);
    let filter = dict.get(b"Filter").unwrap();
    assert_eq!(filter.as_name_str().unwrap(), "FlateDecode");
    let cs = dict.get(b"ColorSpace").unwrap();
    assert_eq!(cs.as_name_str().unwrap(), "DeviceRGB");

    // verify SMask is present (alpha channel)
    let smask = dict.get(b"SMask");
    assert!(smask.is_ok(), "RGBA image should have SMask for alpha channel");
}

#[test]
fn test_merge_png_palette() {
    let dir = tmp_dir("png_palette");
    let img = dir.join("test.png");
    let pdf = dir.join("out.pdf");
    write_tiny_png_palette(&img);
    run_merge(&[img], &pdf);

    let doc = lopdf::Document::load(&pdf).unwrap();
    assert_eq!(doc.get_pages().len(), 1);

    let dict = get_first_page_image_dict(&doc);
    let filter = dict.get(b"Filter").unwrap();
    assert_eq!(filter.as_name_str().unwrap(), "FlateDecode");

    // color space should be an array: [/Indexed /DeviceRGB N <palette>]
    let cs = dict.get(b"ColorSpace").unwrap();
    match cs {
        lopdf::Object::Array(arr) => {
            assert_eq!(arr.len(), 4, "Indexed color space should have 4 elements");
            assert_eq!(arr[0].as_name_str().unwrap(), "Indexed");
            assert_eq!(arr[1].as_name_str().unwrap(), "DeviceRGB");
            // arr[2] is max index (num_entries - 1)
            let max_idx = arr[2].as_i64().unwrap();
            assert_eq!(max_idx, 3); // 4 palette entries -> max index 3
        }
        _ => panic!("Expected array color space for palette PNG"),
    }
}

#[test]
fn test_merge_multiple_images() {
    let dir = tmp_dir("multi");
    let jpg = dir.join("a.jpg");
    let png_rgb = dir.join("b.png");
    let png_rgba = dir.join("c.png");
    let pdf = dir.join("out.pdf");

    write_tiny_jpeg_rgb(&jpg);
    write_tiny_png_rgb(&png_rgb);
    write_tiny_png_rgba(&png_rgba);

    run_merge(&[jpg, png_rgb, png_rgba], &pdf);

    let doc = lopdf::Document::load(&pdf).unwrap();
    assert_eq!(doc.get_pages().len(), 3);
}

#[test]
fn test_roundtrip_split_merge() {
    // pick the first available test PDF
    let test_data = PathBuf::from("test/data");
    if !test_data.exists() {
        eprintln!("Skipping roundtrip test: test/data directory not found");
        return;
    }
    let pdf_entry = std::fs::read_dir(&test_data)
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.path().extension().is_some_and(|ext| ext == "pdf"));
    let source_pdf = match pdf_entry {
        Some(e) => e.path(),
        None => {
            eprintln!("Skipping roundtrip test: no PDF in test/data");
            return;
        }
    };

    // count pages in source
    let source_doc = lopdf::Document::load(&source_pdf).unwrap();
    let source_pages = source_doc.get_pages().len();
    drop(source_doc);

    let dir = tmp_dir("roundtrip");
    let split_dir = dir.join("pages");
    std::fs::create_dir_all(&split_dir).unwrap();

    // split: PDF -> PNGs
    let output = Command::new(ovid_bin())
        .args(["split", source_pdf.to_str().unwrap(), "-o"])
        .arg(&split_dir)
        .arg("--quiet")
        .output()
        .expect("failed to run ovid split");
    if !output.status.success() {
        panic!(
            "ovid split failed:\nstderr: {}",
            String::from_utf8_lossy(&output.stderr),
        );
    }

    // collect PNGs in sorted order
    let mut pngs: Vec<PathBuf> = std::fs::read_dir(&split_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "png"))
        .collect();
    pngs.sort();

    assert_eq!(
        pngs.len(),
        source_pages,
        "Split should produce one PNG per page"
    );

    // merge: PNGs -> PDF
    let merged_pdf = dir.join("merged.pdf");
    run_merge(&pngs, &merged_pdf);

    let merged_doc = lopdf::Document::load(&merged_pdf).unwrap();
    assert_eq!(
        merged_doc.get_pages().len(),
        source_pages,
        "Merged PDF should have same page count as source"
    );
}
