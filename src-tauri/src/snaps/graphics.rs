//! One-shot HWND capture via Windows Graphics Capture. Unlike screen copying,
//! this captures the terminal's surface rather than overlapping applications.
use super::model::Bounds;
use image::{Rgba, RgbaImage};
use std::path::Path;
use std::time::{Duration, Instant};
use windows::core::Interface;
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat};
use windows::Win32::Foundation::{HMODULE, HWND, RECT};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

pub struct CaptureDevice {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    winrt_device: IDirect3DDevice,
}

struct Session {
    session: GraphicsCaptureSession,
    pool: Direct3D11CaptureFramePool,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

impl CaptureDevice {
    pub fn new() -> Result<Self, String> {
        unsafe {
            if !GraphicsCaptureSession::IsSupported().map_err(|e| e.to_string())? {
                return Err(
                    "Windows Graphics Capture is unavailable (Windows 10 1903+ required)".into(),
                );
            }
            let mut device = None;
            let mut context = None;
            let mut result = Err(windows::core::Error::from_thread());
            for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
                result = D3D11CreateDevice(
                    None,
                    driver,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                );
                if result.is_ok() {
                    break;
                }
            }
            result.map_err(|e| format!("Create capture device: {e}"))?;
            let device = device.ok_or("No capture device")?;
            let context = context.ok_or("No capture context")?;
            let dxgi: IDXGIDevice = device.cast().map_err(|e| e.to_string())?;
            let winrt_device = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)
                .and_then(|v| v.cast())
                .map_err(|e| e.to_string())?;
            Ok(Self {
                device,
                context,
                winrt_device,
            })
        }
    }

    pub fn save(
        &self,
        hwnd: HWND,
        viewport: Option<Bounds>,
        destination: &Path,
    ) -> Result<(), String> {
        self.capture(hwnd, viewport)
            .map_err(|e| format!("Capture image: {e}"))?
            .save(destination)
            .map_err(|e| format!("Save image: {e}"))
    }

    fn capture(&self, hwnd: HWND, viewport: Option<Bounds>) -> Result<RgbaImage, String> {
        let interop: IGraphicsCaptureItemInterop =
            windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
                .map_err(|e| e.to_string())?;
        let item: GraphicsCaptureItem =
            unsafe { interop.CreateForWindow(hwnd) }.map_err(|e| e.to_string())?;
        let size = item.Size().map_err(|e| e.to_string())?;
        if size.Width <= 0 || size.Height <= 0 {
            return Err("Window has no drawable area".into());
        }
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &self.winrt_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            size,
        )
        .map_err(|e| e.to_string())?;
        let session = match pool.CreateCaptureSession(&item) {
            Ok(session) => session,
            Err(error) => {
                let _ = pool.Close();
                return Err(error.to_string());
            }
        };
        let capture = Session { session, pool };
        let _ = capture.session.SetIsCursorCaptureEnabled(false);
        capture.session.StartCapture().map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(3);
        let frame = loop {
            if let Ok(frame) = capture.pool.TryGetNextFrame() {
                break frame;
            }
            if Instant::now() >= deadline {
                return Err("Timed out waiting for a window frame".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let result = self.read_frame(&frame, hwnd, viewport);
        let _ = frame.Close();
        result
    }

    fn read_frame(
        &self,
        frame: &Direct3D11CaptureFrame,
        hwnd: HWND,
        viewport: Option<Bounds>,
    ) -> Result<RgbaImage, String> {
        unsafe {
            let size = frame.ContentSize().map_err(|e| e.to_string())?;
            let access: IDirect3DDxgiInterfaceAccess = frame
                .Surface()
                .and_then(|s| s.cast())
                .map_err(|e| e.to_string())?;
            let texture: ID3D11Texture2D = access.GetInterface().map_err(|e| e.to_string())?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);
            let width = u32::try_from(size.Width).map_err(|_| "Invalid frame width")?;
            let height = u32::try_from(size.Height).map_err(|_| "Invalid frame height")?;
            if width == 0 || height == 0 || width > desc.Width || height > desc.Height {
                return Err("Window resized during capture; take another snapshot".into());
            }
            desc.Usage = D3D11_USAGE_STAGING;
            desc.BindFlags = 0;
            desc.MiscFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
            let mut staging = None;
            self.device
                .CreateTexture2D(&desc, None, Some(&mut staging))
                .map_err(|e| e.to_string())?;
            let staging = staging.ok_or("No staging texture")?;
            self.context.CopyResource(&staging, &texture);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(|e| e.to_string())?;
            // Own the bytes before unmapping. GPU rows can contain padding.
            let mut image = RgbaImage::new(width, height);
            for y in 0..height {
                let row = std::slice::from_raw_parts(
                    (mapped.pData as *const u8).add(y as usize * mapped.RowPitch as usize),
                    width as usize * 4,
                );
                for (x, pixel) in row.chunks_exact(4).enumerate() {
                    image.put_pixel(x as u32, y, Rgba([pixel[2], pixel[1], pixel[0], 255]));
                }
            }
            self.context.Unmap(&staging, 0);
            if let Some(viewport) = viewport {
                let mut window = RECT::default();
                if DwmGetWindowAttribute(
                    hwnd,
                    DWMWA_EXTENDED_FRAME_BOUNDS,
                    &mut window as *mut _ as *mut _,
                    std::mem::size_of::<RECT>() as u32,
                )
                .is_err()
                {
                    GetWindowRect(hwnd, &mut window).map_err(|e| e.to_string())?;
                }
                let x = (viewport.left - window.left).clamp(0, width as i32) as u32;
                let y = (viewport.top - window.top).clamp(0, height as i32) as u32;
                let right = (viewport.right - window.left).clamp(0, width as i32) as u32;
                let bottom = (viewport.bottom - window.top).clamp(0, height as i32) as u32;
                if right <= x || bottom <= y {
                    return Err("Viewport moved outside the captured window".into());
                }
                return Ok(
                    image::imageops::crop_imm(&image, x, y, right - x, bottom - y).to_image(),
                );
            }
            Ok(image)
        }
    }
}
