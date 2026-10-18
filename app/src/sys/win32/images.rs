//! Image previews (import.md#design) with WIC, the Windows Imaging
//! Component: an image decoded, turned upright by its EXIF orientation,
//! scaled down to fit a box, and encoded as JPEG for the page. WIC reads
//! whatever codecs Windows has (JPEG, PNG, GIF, BMP, TIFF; WebP and HEIC
//! with their extensions).

use std::path::Path;

use av_utils::error::Result;
use windows::Win32::Foundation::{E_OUTOFMEMORY, E_POINTER, GENERIC_READ};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatJpeg, GUID_WICPixelFormat24bppBGR,
    IWICBitmapFrameDecode, IWICBitmapSource, IWICImagingFactory, IWICPalette,
    WICBitmapDitherTypeNone, WICBitmapEncoderNoCache, WICBitmapInterpolationModeFant,
    WICBitmapPaletteTypeCustom, WICBitmapTransformFlipHorizontal, WICBitmapTransformFlipVertical,
    WICBitmapTransformOptions, WICBitmapTransformRotate0, WICBitmapTransformRotate90,
    WICBitmapTransformRotate180, WICBitmapTransformRotate270, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::StructuredStorage::{
    IPropertyBag2, PROPVARIANT, PropVariantClear,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
    IStream, STREAM_SEEK_END, STREAM_SEEK_SET,
};
use windows::Win32::System::Variant::VT_UI2;
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows::core::{GUID, HSTRING, Interface, w};

use crate::sys::fit;

// ----------------------------------------------------------------------------
/// COM on the current thread while this lives: a worker that uses WIC
/// holds one (the UI thread has its own apartment).
pub struct ComThread {
    initialized: bool,
}

impl ComThread {
    // ------------------------------------------------------------------------
    pub fn new() -> Self {
        let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        Self { initialized }
    }
}

impl Drop for ComThread {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { CoUninitialize() };
        }
    }
}

// ----------------------------------------------------------------------------
/// The image `path` as a JPEG of at most `max_w` x `max_h` pixels, upright
/// by its EXIF orientation, keeping its aspect; never scaled up. Needs COM
/// on the thread (`ComThread`).
pub fn preview(path: &Path, max_w: u32, max_h: u32) -> Result<Vec<u8>> {
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let decoder = factory.CreateDecoderFromFilename(
            &HSTRING::from(path),
            None,
            GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )?;
        let frame = decoder.GetFrame(0)?;
        let (transform, turned) = upright(orientation(&frame));
        let (mut width, mut height) = (0, 0);
        frame.GetSize(&mut width, &mut height)?;
        // Fit the image as it will stand: a turned one swaps the box.
        let (box_w, box_h) = if turned {
            (max_h, max_w)
        } else {
            (max_w, max_h)
        };
        let (width, height) = fit(width, height, box_w, box_h);

        let scaler = factory.CreateBitmapScaler()?;
        scaler.Initialize(&frame, width, height, WICBitmapInterpolationModeFant)?;
        let rotator = factory.CreateBitmapFlipRotator()?;
        rotator.Initialize(&scaler, transform)?;
        let converter = factory.CreateFormatConverter()?;
        converter.Initialize(
            &rotator,
            &GUID_WICPixelFormat24bppBGR,
            WICBitmapDitherTypeNone,
            None::<&IWICPalette>,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;
        encode(&factory, &converter.cast()?, &GUID_ContainerFormatJpeg)
    }
}

// ----------------------------------------------------------------------------
/// The EXIF orientation of `frame` (`System.Photo.Orientation`, 1 to 8);
/// 1, upright, if it has none.
unsafe fn orientation(frame: &IWICBitmapFrameDecode) -> u16 {
    unsafe {
        let Ok(reader) = frame.GetMetadataQueryReader() else {
            return 1;
        };
        let mut value = PROPVARIANT::default();
        if reader
            .GetMetadataByName(w!("System.Photo.Orientation"), &mut value)
            .is_err()
        {
            return 1;
        }
        let inner = &value.Anonymous.Anonymous;
        let orientation = if inner.vt == VT_UI2 {
            inner.Anonymous.uiVal
        } else {
            1
        };
        let _ = PropVariantClear(&mut value);
        orientation
    }
}

// ----------------------------------------------------------------------------
/// What turns an image with the EXIF `orientation` upright (WIC flips
/// first, then rotates clockwise), and whether that swaps its width and
/// height.
fn upright(orientation: u16) -> (WICBitmapTransformOptions, bool) {
    let flipped = |rotation: WICBitmapTransformOptions| {
        WICBitmapTransformOptions(rotation.0 | WICBitmapTransformFlipHorizontal.0)
    };
    match orientation {
        2 => (WICBitmapTransformFlipHorizontal, false),
        3 => (WICBitmapTransformRotate180, false),
        4 => (WICBitmapTransformFlipVertical, false),
        5 => (flipped(WICBitmapTransformRotate270), true),
        6 => (WICBitmapTransformRotate90, true),
        7 => (flipped(WICBitmapTransformRotate90), true),
        8 => (WICBitmapTransformRotate270, true),
        _ => (WICBitmapTransformRotate0, false),
    }
}

// ----------------------------------------------------------------------------
/// `source` (24bppBGR) encoded in `format`, as bytes.
unsafe fn encode(
    factory: &IWICImagingFactory,
    source: &IWICBitmapSource,
    format: &GUID,
) -> Result<Vec<u8>> {
    unsafe {
        let stream = SHCreateMemStream(None).ok_or(windows::core::Error::from(E_OUTOFMEMORY))?;
        let encoder = factory.CreateEncoder(format, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame = None;
        encoder.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
        let frame = frame.ok_or(windows::core::Error::from(E_POINTER))?;
        frame.Initialize(None::<&IPropertyBag2>)?;
        let (mut width, mut height) = (0, 0);
        source.GetSize(&mut width, &mut height)?;
        frame.SetSize(width, height)?;
        let mut pixel_format = GUID_WICPixelFormat24bppBGR;
        frame.SetPixelFormat(&mut pixel_format)?;
        frame.WriteSource(source, std::ptr::null())?;
        frame.Commit()?;
        encoder.Commit()?;
        read_all(&stream)
    }
}

// ----------------------------------------------------------------------------
/// Everything written to `stream`.
unsafe fn read_all(stream: &IStream) -> Result<Vec<u8>> {
    unsafe {
        let mut end = 0u64;
        stream.Seek(0, STREAM_SEEK_END, Some(&mut end))?;
        stream.Seek(0, STREAM_SEEK_SET, None)?;
        let mut data = vec![0u8; end as usize];
        let mut read = 0u32;
        stream
            .Read(data.as_mut_ptr().cast(), data.len() as u32, Some(&mut read))
            .ok()?;
        data.truncate(read as usize);
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use windows::Win32::Graphics::Imaging::{GUID_ContainerFormatPng, GUID_WICPixelFormat8bppGray};

    fn factory() -> IWICImagingFactory {
        unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).unwrap() }
    }

    /// The size of the image in `bytes`.
    fn size_of(bytes: &[u8]) -> (u32, u32) {
        unsafe {
            let stream = SHCreateMemStream(Some(bytes)).unwrap();
            let decoder = factory()
                .CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)
                .unwrap();
            let (mut width, mut height) = (0, 0);
            decoder
                .GetFrame(0)
                .unwrap()
                .GetSize(&mut width, &mut height)
                .unwrap();
            (width, height)
        }
    }

    /// A gray `width` x `height` PNG file.
    fn png(width: u32, height: u32) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "avelor-image-test-{}.png",
            av_utils::id::Id::generate()
        ));
        unsafe {
            let factory = factory();
            let pixels = vec![128u8; (width * height * 3) as usize];
            let bitmap = factory
                .CreateBitmapFromMemory(
                    width,
                    height,
                    &GUID_WICPixelFormat24bppBGR,
                    width * 3,
                    &pixels,
                )
                .unwrap();
            let bytes =
                encode(&factory, &bitmap.cast().unwrap(), &GUID_ContainerFormatPng).unwrap();
            fs::write(&path, bytes).unwrap();
        }
        path
    }

    // ------------------------------------------------------------------------
    #[test]
    fn previews_are_jpegs_that_fit() {
        let _com = ComThread::new();
        let (large, small) = (png(400, 200), png(100, 50));
        let large_preview = preview(&large, 360, 508);
        let small_preview = preview(&small, 360, 508);
        let missing = preview(&large.with_extension("gone"), 360, 508);
        let _ = (fs::remove_file(&large), fs::remove_file(&small));

        let large_preview = large_preview.unwrap();
        assert_eq!(&large_preview[..2], [0xFF, 0xD8], "a JPEG");
        assert_eq!(size_of(&large_preview), (360, 180));
        assert_eq!(size_of(&small_preview.unwrap()), (100, 50));
        assert!(missing.is_err());
    }

    // ------------------------------------------------------------------------
    /// The table matches what the orientations mean, as WIC applies it: a
    /// 2x2 image `[A B; C D]` turned by each orientation's transform.
    #[test]
    fn orientations_turn_images_upright() {
        let _com = ComThread::new();
        let factory = factory();
        let turn = |orientation: u16, stored: [u8; 4]| unsafe {
            let bitmap = factory
                .CreateBitmapFromMemory(2, 2, &GUID_WICPixelFormat8bppGray, 2, &stored)
                .unwrap();
            let rotator = factory.CreateBitmapFlipRotator().unwrap();
            rotator.Initialize(&bitmap, upright(orientation).0).unwrap();
            let mut out = [0u8; 4];
            rotator.CopyPixels(std::ptr::null(), 2, &mut out).unwrap();
            out
        };
        let upright_image = [1, 2, 3, 4]; // [A B; C D] as it should look
        // How each orientation stores that image (EXIF: where row 0 and
        // column 0 of the stored image are when shown).
        let stored = [
            (1, [1, 2, 3, 4]),
            (2, [2, 1, 4, 3]), // mirrored
            (3, [4, 3, 2, 1]), // upside down
            (4, [3, 4, 1, 2]), // flipped
            (5, [1, 3, 2, 4]), // transposed
            (6, [2, 4, 1, 3]), // turned 90 counterclockwise
            (7, [4, 2, 3, 1]), // transposed the other way
            (8, [3, 1, 4, 2]), // turned 90 clockwise
        ];
        for (orientation, image) in stored {
            assert_eq!(
                turn(orientation, image),
                upright_image,
                "orientation {orientation}"
            );
        }
        assert!(!upright(1).1 && !upright(3).1 && upright(6).1 && upright(5).1);
    }
}
