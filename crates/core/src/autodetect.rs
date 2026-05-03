use opencv::{
    core::{self, Mat, Scalar, Vector},
    imgcodecs, imgproc,
    prelude::*,
};
use rayon::prelude::*;
use std::path::Path;

use crate::domain::crop::CropBox;
use crate::domain::values::Rotation;

/// Max dimension for the image during the detection phase.
/// Using a smaller size (1000px) significantly speeds up processing without
/// sacrificing detection accuracy for large shapes like pages.
const PROCESSING_SIZE: u32 = 1000;

/// Represents a group of pages that share similar dimensions.
pub struct CropCluster {
    pub preset: crate::domain::crop::CropPreset,
    pub page_indices: Vec<usize>,
}

/// Detects the visible page boundary in a single image.
/// Returns the detected CropBox and the dimensions of the rotated image.
fn detect_single_fast(path: &Path, page_rotation: Rotation) -> Option<(CropBox, (u32, u32))> {
    // 1. Load directly into OpenCV Mat (Grayscale)
    let path_str = path.to_str()?;
    let src_gray = imgcodecs::imread(path_str, imgcodecs::IMREAD_GRAYSCALE).ok()?;
    if src_gray.empty() {
        return None;
    }

    let orig_w = src_gray.cols();
    let orig_h = src_gray.rows();

    // 2. Fast downscale using OpenCV
    // We downscale while maintaining aspect ratio to keep processing under 1s per page.
    let scale = (PROCESSING_SIZE as f64 / orig_w.max(orig_h) as f64).min(1.0);
    let mut small_gray = Mat::default();
    if scale < 1.0 {
        let new_size = core::Size::new(
            (orig_w as f64 * scale) as i32,
            (orig_h as f64 * scale) as i32,
        );
        imgproc::resize(
            &src_gray,
            &mut small_gray,
            new_size,
            0.0,
            0.0,
            imgproc::INTER_AREA,
        )
        .ok()?;
    } else {
        small_gray = src_gray;
    }

    // 3. Apply User Rotation to the SMALL image
    // This is critical so that centrality and aspect ratio scoring match the final output.
    let rotated_small = match page_rotation {
        Rotation::ZERO => small_gray,
        Rotation::DEG90 => {
            let mut dst = Mat::default();
            core::rotate(&small_gray, &mut dst, core::ROTATE_90_CLOCKWISE).ok()?;
            dst
        }
        Rotation::DEG180 => {
            let mut dst = Mat::default();
            core::rotate(&small_gray, &mut dst, core::ROTATE_180).ok()?;
            dst
        }
        Rotation::DEG270 => {
            let mut dst = Mat::default();
            core::rotate(&small_gray, &mut dst, core::ROTATE_90_COUNTERCLOCKWISE).ok()?;
            dst
        }
        _ => small_gray,
    };

    let d_cols = rotated_small.cols();
    let d_rows = rotated_small.rows();

    // 4. Pre-processing: Blur
    // Reduces sensor noise and halftone textures in the paper.
    let mut blurred = Mat::default();
    imgproc::gaussian_blur(
        &rotated_small,
        &mut blurred,
        core::Size::new(5, 5),
        0.0,
        0.0,
        core::BorderTypes::BORDER_REPLICATE as i32,
        core::AlgorithmHint::ALGO_HINT_DEFAULT,
    )
    .ok()?;

    // 5. Strategy: Dual Engine (Otsu Threshold + Canny Edges)
    // Otsu identifies the solid mass of the paper.
    let mut thresh = Mat::default();
    imgproc::threshold(
        &blurred,
        &mut thresh,
        0.0,
        255.0,
        imgproc::THRESH_BINARY | imgproc::THRESH_OTSU,
    )
    .ok()?;

    // Heuristic: If corners are white, Otsu likely made background white. Invert it.
    // We want the candidate (page) to be the white/foreground area.
    let mut corner_white_count = 0;
    for pt in &[
        core::Point::new(0, 0),
        core::Point::new(d_cols - 1, 0),
        core::Point::new(0, d_rows - 1),
        core::Point::new(d_cols - 1, d_rows - 1),
    ] {
        if let Ok(val) = thresh.at_2d::<u8>(pt.y, pt.x) {
            if *val == 255 {
                corner_white_count += 1;
            }
        }
    }
    if corner_white_count >= 3 {
        let mut inverted = Mat::default();
        core::bitwise_not(&thresh, &mut inverted, &core::no_array()).ok()?;
        thresh = inverted;
    }

    // Canny finds the sharp physical boundaries.
    let mut edges = Mat::default();
    imgproc::canny(&blurred, &mut edges, 30.0, 100.0, 3, false).ok()?;

    let mut combined = Mat::default();
    core::bitwise_or(&thresh, &edges, &mut combined, &core::no_array()).ok()?;

    // SEAL: Draw 1px border to close cut-off page boundaries.
    // This allows open contours (from edges going out of frame) to be detected as closed shapes.
    imgproc::rectangle(
        &mut combined,
        core::Rect::new(0, 0, d_cols, d_rows),
        Scalar::all(255.0),
        1,
        imgproc::LINE_8,
        0,
    )
    .ok()?;

    // Morphology: Closing operation to join edges and fill small internal gaps.
    let mut closed = Mat::default();
    let kernel = imgproc::get_structuring_element(
        imgproc::MORPH_RECT,
        core::Size::new(9, 9),
        core::Point::new(-1, -1),
    )
    .ok()?;
    imgproc::morphology_ex(
        &combined,
        &mut closed,
        imgproc::MORPH_CLOSE,
        &kernel,
        core::Point::new(-1, -1),
        1,
        core::BorderTypes::BORDER_CONSTANT as i32,
        Scalar::all(0.0),
    )
    .ok()?;

    // 7. Contour Detection
    let mut contours = Vector::<Vector<core::Point>>::new();
    // Using RETR_LIST because the 1px border makes the whole image the outer contour.
    // The actual page and background gaps will be inner contours.
    imgproc::find_contours(
        &closed,
        &mut contours,
        imgproc::RETR_LIST,
        imgproc::CHAIN_APPROX_SIMPLE,
        core::Point::new(0, 0),
    )
    .ok()?;

    let total_area = (d_cols * d_rows) as f64;
    let min_area = total_area * 0.05;
    let max_area = total_area * 0.99;

    let center_x = d_cols as f64 / 2.0;
    let center_y = d_rows as f64 / 2.0;

    let mut best_candidate: Option<(f64, core::Rect)> = None;

    // Scoring Loop: Find the shape that looks most like a page.
    for i in 0..contours.len() {
        let contour = contours.get(i).ok()?;
        let area = imgproc::contour_area(&contour, false).ok()?;

        // Basic area filters.
        if area < min_area || area > max_area {
            continue;
        }

        let rect = imgproc::bounding_rect(&contour).ok()?;
        let rect_area = (rect.width * rect.height) as f64;
        let rectangularity = area / rect_area;

        // Candidate must be somewhat rectangular.
        if rectangularity > 0.45 {
            let rect_center_x = rect.x as f64 + (rect.width as f64 / 2.0);
            let rect_center_y = rect.y as f64 + (rect.height as f64 / 2.0);

            // Centrality Score: Favors objects in the middle of the capture.
            let dx = (rect_center_x - center_x) / center_x;
            let dy = (rect_center_y - center_y) / center_y;
            let dist_factor = (dx * dx + dy * dy).sqrt().min(1.5);
            let centrality_score = 1.0 - (dist_factor / 1.5).powi(2);

            // Aspect Ratio Score: Support everything from Square (1.0) to Tall Books (1.6).
            // We calculate how far the dominant dimension is from this "natural" range.
            let aspect = rect.width as f64 / rect.height as f64;
            let val = aspect.max(1.0 / aspect);
            let aspect_diff = if val > 1.6 { val - 1.6 } else { 0.0 };
            let aspect_score = 1.0 - (aspect_diff * 0.5).min(0.7);

            // Sliver Penalty: Drastically penalize thin bars (like scanner artifacts).
            let width_ratio = rect.width as f64 / d_cols as f64;
            let height_ratio = rect.height as f64 / d_rows as f64;
            let sliver_penalty = if width_ratio < 0.15 || height_ratio < 0.15 {
                0.15
            } else {
                1.0
            };

            // Brightness Score: Soft preference for lighter areas (paper is usually white/light).
            let roi = match Mat::roi(&rotated_small, rect) {
                Ok(r) => r,
                Err(_) => continue,
            };
            let mean_val = match core::mean(&roi, &core::no_array()) {
                Ok(m) => m[0],
                Err(_) => 128.0,
            };
            let brightness_score = (mean_val + 50.0) / 305.0;

            // Final Composite Score: Combines all heuristics.
            let score = (area / total_area)
                * rectangularity.powi(2)
                * centrality_score
                * aspect_score
                * sliver_penalty
                * brightness_score;

            if best_candidate.is_none() || score > best_candidate.as_ref().unwrap().0 {
                best_candidate = Some((score, rect));
            }
        }
    }

    // 8. Map back to original (un-downscaled) coordinates.
    let (final_w, final_h) = if matches!(page_rotation, Rotation::DEG90 | Rotation::DEG270) {
        (orig_h as u32, orig_w as u32)
    } else {
        (orig_w as u32, orig_h as u32)
    };

    let crop = best_candidate
        .map(|(_, rect)| {
            let inv_scale = 1.0 / scale;
            CropBox {
                x: ((rect.x as f64 * inv_scale) as u32).min(final_w.saturating_sub(1)),
                y: ((rect.y as f64 * inv_scale) as u32).min(final_h.saturating_sub(1)),
                w: ((rect.width as f64 * inv_scale) as u32).min(final_w),
                h: ((rect.height as f64 * inv_scale) as u32).min(final_h),
            }
        })
        .unwrap_or(CropBox {
            x: 0,
            y: 0,
            w: final_w,
            h: final_h,
        });

    Some((crop, (final_w, final_h)))
}

/// Orchestrates detection across multiple pages using Rayon for parallel processing.
pub fn detect_all_pages(paths: &[(usize, &Path, Rotation)]) -> Vec<(usize, CropBox, u32, u32)> {
    tracing::info!("autodetect: starting detection on {} pages", paths.len());

    paths
        .par_iter()
        .filter_map(|&(idx, path, rotation)| {
            let (crop, (w, h)) = detect_single_fast(path, rotation).unwrap_or_else(|| {
                // Fallback: Return the full image size if detection fails.
                let src =
                    imgcodecs::imread(path.to_str().unwrap_or(""), imgcodecs::IMREAD_UNCHANGED)
                        .ok();
                if let Some(s) = src {
                    let (w, h) = if matches!(rotation, Rotation::DEG90 | Rotation::DEG270) {
                        (s.rows() as u32, s.cols() as u32)
                    } else {
                        (s.cols() as u32, s.rows() as u32)
                    };
                    (CropBox { x: 0, y: 0, w, h }, (w, h))
                } else {
                    (
                        CropBox {
                            x: 0,
                            y: 0,
                            w: 0,
                            h: 0,
                        },
                        (0, 0),
                    )
                }
            });

            Some((idx, crop, w, h))
        })
        .collect()
}

/// Groups detected crop boxes into clusters based on aspect ratio and area similarity.
/// This creates shared presets that minimize the impact of individual detection noise.
pub fn cluster_by_similarity(crops: &[(usize, CropBox)]) -> Vec<CropCluster> {
    if crops.is_empty() {
        return Vec::new();
    }
    if crops.len() == 1 {
        let (idx, cb) = &crops[0];
        return vec![CropCluster {
            preset: crate::domain::crop::CropPreset {
                name: "Preset 1".into(),
                w: cb.w,
                h: cb.h,
                locked: false,
            },
            page_indices: vec![*idx],
        }];
    }

    let ratios: Vec<f64> = crops
        .iter()
        .map(|(_, cb)| {
            if cb.h > 0 {
                cb.w as f64 / cb.h as f64
            } else {
                1.0
            }
        })
        .collect();

    let mut assigned: Vec<bool> = vec![false; crops.len()];
    let mut groups: Vec<Vec<usize>> = Vec::new();

    for i in 0..crops.len() {
        if assigned[i] {
            continue;
        }
        let mut group = vec![i];
        assigned[i] = true;
        for j in (i + 1)..crops.len() {
            if assigned[j] {
                continue;
            }
            let (a, b) = (ratios[i], ratios[j]);
            let ratio_sim = if a > b { b / a } else { a / b };

            let cb_i = &crops[i].1;
            let cb_j = &crops[j].1;
            let area_i = cb_i.w as f64 * cb_i.h as f64;
            let area_j = cb_j.w as f64 * cb_j.h as f64;
            let area_sim = if area_i > area_j {
                area_j / area_i
            } else {
                area_i / area_j
            };

            // Group only if aspect ratio and area are extremely similar (>95% and >80%).
            if ratio_sim > 0.95 && area_sim > 0.8 {
                group.push(j);
                assigned[j] = true;
            }
        }
        groups.push(group);
    }

    groups.sort_by_key(|g| -(g.len() as i64));

    let mut clusters = Vec::new();
    for (gi, group) in groups.iter().enumerate() {
        let mut ws: Vec<u32> = group.iter().map(|&i| crops[i].1.w).collect();
        let mut hs: Vec<u32> = group.iter().map(|&i| crops[i].1.h).collect();
        ws.sort_unstable();
        hs.sort_unstable();

        // Final preset size is the MEDIAN of the cluster to filter out outliers.
        let n = ws.len();
        let median_w = if n % 2 == 1 {
            ws[n / 2]
        } else {
            (ws[n / 2 - 1] + ws[n / 2]) / 2
        };
        let median_h = if n % 2 == 1 {
            hs[n / 2]
        } else {
            (hs[n / 2 - 1] + hs[n / 2]) / 2
        };
        let name = format!("Preset {}", gi + 1);

        clusters.push(CropCluster {
            preset: crate::domain::crop::CropPreset {
                name,
                w: median_w,
                h: median_h,
                locked: false,
            },
            page_indices: group.iter().map(|&i| crops[i].0).collect(),
        });
    }

    clusters
}
