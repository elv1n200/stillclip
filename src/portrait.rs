use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Crop {
    pub x: f64,
    pub y: f64,
    pub width: u32,
}
impl Crop {
    pub fn validate(self) -> Result<Self, String> {
        if !self.x.is_finite()
            || !self.y.is_finite()
            || !(0.0..=1.0).contains(&self.x)
            || !(0.0..=1.0).contains(&self.y)
            || ![720, 1080].contains(&self.width)
        {
            return Err(crate::i18n::text("Der Hochkant-Ausschnitt ist ungültig.", "The portrait crop is invalid.").into());
        }
        Ok(self)
    }
    pub fn height(self) -> u32 {
        self.width * 16 / 9
    }
    pub fn filter(self) -> String {
        format!("crop=w='max(2,trunc(min(iw,ih*9/16)/2)*2)':h='max(2,trunc(min(ih,iw*16/9)/2)*2)':x='trunc((iw-ow)*{:.6}/2)*2':y='trunc((ih-oh)*{:.6}/2)*2',scale={}:{},setsar=1", self.x, self.y, self.width, self.height())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires FFmpeg; generated color video only"]
    fn synthetic_portrait_exports_keep_crop_position_dimensions_and_discord_limit() {
        use crate::{capture::command, discord, highlights, jobs::Control, recorder};
        let ffmpeg = recorder::find_ffmpeg().unwrap();
        let root = std::env::temp_dir().join(format!("stillclip-portrait-{}", std::process::id()));
        let workspace = crate::export::Snapshot::new(&root, &[]).unwrap();
        let input = workspace.dir.join("red-blue.mp4");
        let output = command(&ffmpeg)
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "color=c=red:s=640x360:r=15,drawbox=x=320:y=0:w=320:h=360:color=blue:t=fill",
                "-t",
                "2",
                "-c:v",
                "libx264",
                "-threads",
                "2",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&input)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        for (x, width) in [(0.0, 720), (1.0, 1080)] {
            let output = workspace.dir.join(format!("crop-{width}.mp4"));
            let crop = Crop { x, y: 0.5, width };
            recorder::trim_controlled(
                &ffmpeg,
                &input,
                &output,
                0.0,
                2.0,
                recorder::TrimMode::Fast,
                Some(crop),
                &Control::new(),
            )
            .unwrap();
            let info = recorder::probe_media(&ffmpeg, &output).unwrap();
            assert_eq!((info.width, info.height), (width, crop.height()));
            assert!((info.duration - 2.0).abs() < 0.1);
            let frame = command(&ffmpeg)
                .args(["-v", "error", "-i"])
                .arg(&output)
                .args([
                    "-vf",
                    "scale=1:1",
                    "-frames:v",
                    "1",
                    "-f",
                    "rawvideo",
                    "-pix_fmt",
                    "rgb24",
                    "-",
                ])
                .output()
                .unwrap();
            assert!(frame.status.success());
            assert_eq!(frame.stdout.len(), 3);
            if x == 0.0 {
                assert!(frame.stdout[0] > 150 && frame.stdout[2] < 80);
            } else {
                assert!(frame.stdout[2] > 150 && frame.stdout[0] < 80);
            }
        }
        let crop = Crop {
            x: 1.0,
            y: 0.5,
            width: 720,
        };
        let compressed = workspace.dir.join("discord.mp4");
        let bytes = discord::export_controlled(
            &ffmpeg,
            &input,
            &compressed,
            0.0,
            2.0,
            discord::Profile::Free,
            Some(crop),
            &Control::new(),
        )
        .unwrap();
        assert!(bytes < discord::Profile::Free.limit_bytes());
        let info = recorder::probe_media(&ffmpeg, &compressed).unwrap();
        assert!(info.height > info.width);
        let pieces = [highlights::Piece {
            path: input,
            start: 0.0,
            end: 2.0,
            markers: vec![1.0],
            crop: Some(crop),
        }];
        let output = workspace.dir.join("highlight.mp4");
        let marks = highlights::export_controlled(
            &ffmpeg,
            &pieces,
            &output,
            None,
            Some(720),
            &Control::new(),
        )
        .unwrap();
        assert_eq!(marks, [1.0]);
        let info = recorder::probe_media(&ffmpeg, &output).unwrap();
        assert_eq!((info.width, info.height), (720, 1280));
        assert!(info.audio);
        drop(workspace);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn crop_rejects_nonfinite_coordinates_and_unsupported_sizes() {
        assert!(Crop {
            x: 0.5,
            y: 0.5,
            width: 1080
        }
        .validate()
        .is_ok());
        for x in [-0.1, 1.1, f64::NAN, f64::INFINITY] {
            assert!(Crop {
                x,
                y: 0.5,
                width: 720
            }
            .validate()
            .is_err());
        }
        assert!(Crop {
            x: 0.5,
            y: 0.5,
            width: 1000
        }
        .validate()
        .is_err());
    }
}
