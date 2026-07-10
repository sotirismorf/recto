# Page Auto-Detection Algorithm

Pagecutter2 uses a sophisticated multi-stage computer vision pipeline to automatically detect page boundaries in scanned or photographed images. The algorithm is designed to be robust against varying light conditions, dark pages, scanner artifacts, and images where pages are partially out of frame.

## Core Pipeline (The "Dual Engine" Strategy)

The detection process in `crates/core/src/autodetect.rs` follows these key steps:

### 1. Pre-Processing
*   **Downscaling:** The image is resized to a maximum dimension of 1000px for speed.
*   **Rotation:** User-defined page rotations are applied to the downscaled image *before* detection to ensure the aspect ratio scoring is accurate.
*   **Gaussian Blur:** A 5x5 blur is applied to reduce noise and scanner texture.

### 2. Boundary Identification (The Fusion)
Instead of relying on a single detection method, the algorithm fuses two complementary sources:
*   **Otsu Thresholding:** Automatically calculates an optimal brightness threshold to separate the "solid blob" of the page from the background. It includes a corner-check heuristic to handle both black and white scanner backgrounds.
*   **Canny Edge Detection:** Identifies sharp transitions in intensity to find the precise physical edges of the paper.
*   **Boundary Sealing:** A 1px white border is drawn around the combined mask. This "bridges" any edges that go out of frame, allowing the algorithm to detect cut-off pages as closed rectangles.

### 3. Morphology
*   **Closing:** A 9x9 morphological "close" operation is performed to join nearby edges and fill small internal gaps (like text or small shadows) to create a solid candidate area.

### 4. Candidate Scoring & Selection
The algorithm identifies all potential shapes (`RETR_LIST`) and calculates a composite score for each. This scoring system is what allows it to distinguish between the actual page and artifacts like black scanner bars.

**The Scoring Formula:**
`Score = (Area Ratio) * (Rectangularity²) * Centrality * AspectMatch * SliverPenalty * BrightnessBonus`

*   **Area Ratio:** Prefers larger objects (must be > 5% of image).
*   **Rectangularity:** Measures how well the shape fills its bounding box (favors clean rectangles).
*   **Centrality (Quadratic):** Heavily penalizes objects far from the image center.
*   **AspectMatch:** Supports everything from square paper (1.0) to tall book formats (1.6) with a perfect score. Only extreme "strips" are penalized.
*   **Sliver Penalty:** Drastically slashes the score of any object thinner than 15% of the image size (effectively ignoring thin black side-bars).
*   **Brightness Bonus:** A soft multiplier that favors lighter areas while still allowing detection of dark pages.

## Clustering & Presets
Once all pages are detected, `cluster_by_similarity` groups them into **Presets**.
*   **Strict Grouping:** Clusters are formed only if pages have a >95% aspect ratio similarity AND >80% area similarity.
*   **Median Sizing:** The final preset size is the **median** width and height of all pages in that cluster, which filters out individual detection outliers.
*   **UI Clamping:** When applying a preset, the UI ensures the resulting box is centered on the detection but strictly clamped to the image's physical pixel boundaries.
