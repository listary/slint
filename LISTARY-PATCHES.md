# Listary 对 Slint 1.18.1 的补丁

本分支 `listary/1.18.1` 从 Slint 的 `v1.18.1` 标签拉出，只放 Listary 需要的补丁。Listary 仓库的 `app/Cargo.toml` 用 `[patch.crates-io]` 从这个分支取被补丁的包，版本号不变。每处改动都有 `LISTARY PATCH` 注释。

## `i-slint-core`（`internal/core`）

`internal/core` 相对 v1.18.1 除了下文「上游 PR 回移」那一节，改了两处：文字排版缓存清扫时，保留缓存条目和它的依赖追踪器，只丢掉排好的字形；时钟读数比起点早时不再 panic（见本节末「时钟倒退」）。v1.18.1 的 `internal/core` 与 crates.io 上 `i-slint-core` 1.18.1 发布包的源码逐文件相同（差别只在发布时规范化的 `Cargo.toml`、cargo 生成的元数据和展开的许可证符号链接）。

### 问题

软件渲染按需局部重绘时，一个大小不随文字变化的 `Text`（例如文件搜索窗的标题和状态栏「N 个项目」），在结果列表整批替换几次之后，就不再随文字重画，屏幕上一直留着旧字，直到改窗口大小等操作触发整窗重画。

原因在 `textlayout/sharedparley/cache.rs`：

- `Text` 只在排版时读自己的 `text`，而排版在它的缓存条目的依赖追踪器里进行。所以局部重绘对这个 `Text` 的追踪，只能经由这个条目间接依赖文字。
- 缓存条目估计数超过阈值（1024）时，`TextLayoutCache::sweep` 把最近两帧没用过的条目整个删掉，依赖追踪器也一起删了。删追踪器不会把依赖它的一方标脏，于是从文字到这个 `Text` 重画的那条链断了。之后文字再变，没有任何东西被标脏。
- 宽高跟着文字变的 `Text` 会因几何变化重画，不受影响；结果行每次列表变化都会重画，条目也不会过期。受影响的是大小固定、单独放着、所在窗口又有大量文字元素反复新建的那种。列表每次整批替换，新建的行元素都会新增缓存条目，很快就会触发清扫。

### 改动

两个文件，每处都有 `LISTARY PATCH` 注释，`grep -rn "LISTARY PATCH" internal/core/` 可以一次列全。

| 文件 | 改了什么 |
|---|---|
| `textlayout/sharedparley/cache.rs` | ① `sweep` 不再删条目：对最近两帧没用过的条目，把 `paragraphs` 和 `line_breaking` 置为 `None`，条目和追踪器都留着。② `cached_paragraphs` 遇到没有 `paragraphs` 的条目时，不再像原版那样释放后重建，而是在借用缓存之前重新排版，再把结果放回原条目，追踪器始终不换。原因：释放条目会把新追踪器登记给当时正在求值的绑定；如果那是布局查询而不是元素的绘制，元素的重画就又失去了对文字的依赖。③ 清扫从宽度模式检查之后挪到之前。否则这次清扫可能清空本元素自己的条目，而它的追踪器是干净的，会被当成命中，返回空排版，文字画成空白。④ `paragraphs` 字段的说明补了一句「清扫也会留下 `None`」。 |
| `item_rendering.rs` | `ItemCache::retain` 换成 `for_each_mut`：只能原地改条目的数据，不能删条目。`retain` 在 1.18.1 里只有这次清扫用到。 |

内存上限从「最近两帧用过的条目」变成「每个还活着的文字元素留一个小追踪器，加上最近两帧的字形」。元素所在组件销毁时，条目照旧由 `component_destroyed` 删掉。

与 v1.18.1 的差别：`git diff v1.18.1 -- internal/core` 应只列出这两个文件（回移那一节的改动除外）。

### 测试

- Listary 仓库 `app/crates/listary-ui/tests/text_layout_cache_sweep_keeps_repaint.rs`：最小窗口，一个固定大小的标题（最先画）加 256 行的 `for`。
  - 第一段：条目数恰好在第 4 轮最后一行超过上限，然后只改标题颜色、不改文字。于是标题自己的那次取用触发清扫，清空的正是它自己的条目，而追踪器是干净的。标题必须照常画出文字。清扫若放在检查之后，这里会画成空白。
  - 第二段：再换 10 轮后改 3 次标题文字，标题像素每次都要变。用原版 1.18.1 时第一次改文字就失败。
  - 第三段：再换 10 轮，让标题的条目被清空；先读一次标题的 `preferred-width`（布局查询，不是绘制），再改文字，标题像素要变。若把清空的条目改回「释放后重建」，这一段会失败，因为新追踪器登记给了布局绑定。
- Listary 仓库 `app/crates/listary-ui/tests/fsw_text_repaint_after_cache_sweep.rs`：真实的文件搜索窗，结果集换 30 轮后改查询和状态文字。用原版 1.18.1 时标题停在第一个新查询上。

两条测试都用 `MinimalSoftwareWindow` 加复用缓冲，按 Slint 要求才画帧，和桌面后端的局部重绘一致。它们放在 Listary 仓库里，随应用的测试一起跑。

### 升级

升级 Slint 时先看上游是否已修：`TextLayoutCache::sweep` 如果不再删除带追踪器的条目，或者改由调用方在缓存外读取文字，就可以去掉这份分叉，改回 registry 版本。上面两条测试照常要过。如果还要保留分叉，从新版本的标签拉新分支，再把这两处改动打上去。

### 时钟倒退

`Platform::duration_since_start` 的默认实现（`platform.rs`）断言 `Instant::now()` 不早于第一次读数，否则 panic「The platform's clock is not monotonic!」。Windows 的性能计数器在少数机器上会往回跳一点，有时就在程序刚起来时：Listary 7.1.0.37 里 2 个用户各崩一次，一次在事件循环更新计时器时，一次在启动时建第一个窗口时。

改动：读数比起点早时按「没有经过时间」算（`saturating_duration_since`）。计算挪进一个小函数 `elapsed_since`，带 `LISTARY PATCH (clock-goes-back)` 注释，好写单元测试；`Instant::now()` 本身没法在测试里倒回去。

测试：`platform.rs` 的 `a_clock_that_goes_back_counts_as_no_time_elapsed`（`cargo test -p i-slint-core --lib a_clock_that_goes_back`），改前 panic。

这一处只保证读数不早于起点，不保证不早于上一次。动画驱动 `AnimationDriver::update_animations`（`animations.rs`）还有同一句断言，7.1.0.43、45 上崩在那里（LISTARY-5FF）。第二轮改动：

- `animations::Instant::now(ctx)` 在 `SlintContext` 里记住最新读数，返回「这次和最新取大」。动画、定时器、帧统计都从这里取时刻，时钟倒退时停在原处，等真实时间追上来再走，停的时长等于倒退量。各上下文各记各的。
- `update_animations` 的断言改成判断：新时刻不晚于当前就不更新。一个线程上有起点不同的几个上下文时，仍会收到更早的时刻。
- 每次读时钟（`animations.rs` 的 `ClockSample::take`）同时记原始读数和「往回跳的次数」：读数比上一次读数早，次数加一；追赶期间读数照常变大，次数不变。
- 双击判断（`input.rs` 的 `ClickState::check_repeat`）记住按下时的读数和次数。两次按下之间往回跳过（次数不同），一律不算双击，哪怕跳回后的读数仍晚于上次按下；次数相同时按原始读数比间隔。用取大后的时刻会把倒退后隔了很久的两次点击当成同一时刻，误认成双击。
- 限制：往回跳只有在时钟超前时有人读过才认得出。读数来自每轮事件循环、每次定时器更新和每次按下；两次按下之间一次都没读过、时钟又先往前再跳回时，和短暂停顿分不出来，按原始间隔判双击。

测试（`cargo test -p i-slint-core --lib -- goes_back went_back`，改前全部 panic）：
- `animations.rs` 的 `instant_now_holds_while_the_clock_goes_back`：倒退 10 秒后停在原值，追上后照常前进；
- `animations.rs` 的 `an_event_loop_turn_after_the_clock_went_back_keeps_animations_and_timers_in_order`：事件循环在倒退后跑一轮不崩，定时器按追上后的时间触发；
- `properties_animations.rs` 的 `properties_test_animation_holds_while_the_tick_goes_back`：动画既不倒退，也不跳到终点；
- `input.rs` 的 `double_click_across_a_clock_that_went_back`：跨过倒退的两次按下不算双击，包括跳回后读数仍晚于上次按下的情况；倒退后、追上之前，间隔内的两次算；倒退后隔 1 秒的两次不算；时钟正常时不变。
- `api/rs/slint/tests/animation_clock_goes_back.rs`：经 `slint::platform::update_timers_and_animations` 走一遍事件循环。

升级：上游 v1.18.1 之后的 master 仍是这两句断言（上游把它当成发现坏时钟的手段，见 slint-ui/slint#3491）。上游不改就把这几处挑过去。

## `i-slint-renderer-software`（`internal/renderers/software`）

`internal/renderers/software` 相对 v1.18.1 除了下文「上游 PR 回移」和「圆角裁剪」两节，改了三处：局部重绘时，`Path` 只画在本次要重画的区域里；窗口尺寸和缓冲区对不上时不再 panic（见本节末「窗口尺寸大过缓冲区」）；超出 16 位坐标的图元画可见部分、不再 panic（见「超出 16 位坐标的图元」）。

### 问题

设置窗口命令页、动作页打开「添加」下拉菜单后，鼠标在菜单项上移动，菜单下面那几行列表的线条图标会浮到菜单上面。

原因在 `RenderToBuffer`：局部重绘会重画所有和重画区域相交的元素。其他元素都经 `foreach_ranges` 只写重画区域里的像素；`process_filled_path` 和 `process_stroked_path` 却把 `Path` 画满它自己的裁剪框。鼠标移到某个菜单项上，只有那一项要重画，但它下面那行的图标与之相交，于是整个图标被重画，压在重画区域外、上一帧已经画好的菜单像素上。

### 改动

两个文件，每处都有 `LISTARY PATCH` 注释，`grep -rn "LISTARY PATCH" internal/renderers/software/` 可以列全：

- `path.rs`：`render_filled_path`、`render_stroked_path` 多收一个重画区域。遮罩照旧按整个路径生成一次，写入像素时只走裁剪框和重画区域的交集，按 `region_line_ranges` 合并后的行区间逐行写，和 `foreach_ranges` 一样。重画区域的矩形可以互相重叠（`internal/core/partial_renderer.rs` 的 `DirtyRegion` 不保证不重叠），合并后每个像素只混合一次；逐个矩形各画一次会让重叠处的半透明路径和抗锯齿边缘混合两遍、颜色变深。
- `lib.rs`：`RenderToBuffer` 的两个路径函数把自己的 `dirty_region` 传下去。

与 v1.18.1 的差别：`git diff v1.18.1 -- internal/renderers/software` 应只列出这两个文件（回移那一节的改动除外）。

### 测试

- 本分支 `internal/renderers/software/lib.rs` 的 `a_path_draws_only_inside_the_dirty_region`：半透明的填充和描边路径，重画区域是左上角两块互相重叠的矩形。区域内每个像素要和整幅重画时完全相同（重叠处只混合一次），区域外一个像素都不能写。用原版 1.18.1 时会写到区域外；改成逐个矩形各画一次时，重叠处会和整幅重画不同。
- Listary 仓库 `app/crates/listary-ui/tests/path_redraw_stays_in_dirty_region.rs`：最小窗口里一个 `Path` 图标被上层矩形盖住，只改上层一个小块的颜色后，图标不能透出来。

### 升级

升级 Slint 时先看上游的 `RenderToBuffer` 画路径时是否已按重画区域裁剪；已经裁剪的，就去掉这处补丁。上面两条测试照常要过。

### 窗口尺寸大过缓冲区

`SoftwareRenderer::render` 按「窗口元素的逻辑尺寸 × 当前缩放」定这一帧的大小，再断言缓冲区装得下，装不下就 panic「buffer of size … is too small to handle a window of size …」；这一步把浮点尺寸转成 16 位整数时，遇到 NaN 或超出范围还会先在 `cast().unwrap()` 上 panic。winit 后端的软件渲染（`renderer/sw.rs`）按原生窗口的物理尺寸给缓冲区，而 Windows 的 DPI 变化是先报新缩放、后报新尺寸：

- 两者之间来一次 `WM_PAINT`，就按旧逻辑尺寸 × 新缩放要画面，缓冲区还是旧物理尺寸。
- 最大化或全屏的窗口，winit 处理 `WM_DPICHANGED` 时不改窗口大小（`allow_resize` 为假），不会再有 `Resized`；winit 后端的 `ScaleFactorChanged` 分支也不重设逻辑尺寸（上游那里是一句 TODO）。这样的窗口此后每次重绘都对不上。

Listary 7.1.0.37 里断言 4 次、转换 3 次，共 4 个用户；倍数正好是一次缩放变化（160×28 → 240×42 是 ×1.5，2560×1368 → 3072×1641 是 ×1.2），两例接近整块工作区，像最大化的窗口。

改动（`lib.rs` 的 `render_buffer_impl`，`LISTARY PATCH (render-size-fits-buffer)`）：

- 帧尺寸按缓冲区的宽高截断，只画装得下的部分；尺寸跟上之后的下一帧照常整窗画。
- 尺寸是 NaN 或超出 16 位范围时跳过这一帧：后面画各个元素时同样的转换也会失败。

没改的：最大化窗口在 DPI 变化后，逻辑尺寸仍是旧的，所以画出来的是按旧逻辑尺寸排版、截到缓冲区的画面，直到窗口再改大小。根治要在 winit 后端的 `ScaleFactorChanged` 分支里按当前物理尺寸重设逻辑尺寸，那需要真窗口最大化后再测，没放在这里。

测试：`api/rs/slint/tests/render_size_fits_buffer.rs`（`cargo test -p slint --no-default-features --features std,compat-1-2,renderer-software --test render_size_fits_buffer`）。`MinimalSoftwareWindow` 160×28：先发缩放 1.5、不改尺寸，160×28 的缓冲区要整幅画满；再发 40000 宽和 NaN 的尺寸，不 panic、不画；尺寸恢复后整幅画满。改前前两种分别 panic 在断言和转换上，消息和线上相同。

这个测试不能带默认特性跑：slint 的开发依赖给软件渲染器开了 `testing` 特性，它的同名公开方法 `text_layout_cache` 会挡住 `sw.rs` 里对 `RendererSealed::text_layout_cache` 的调用，winit 后端编译不过。这是本分支原有的问题，与这处补丁无关。

升级：上游 master 仍是这句断言。上游不改就把这处挑过去。

### 超出 16 位坐标的图元

软件渲染器的屏幕坐标是 16 位整数（±32767 像素）。几处在和裁剪区求交之前就把整块几何 `cast()` 成 16 位，转不了就在 euclid 里 `unwrap` panic。搜索框里选中很长的一行（100% 缩放下三千多个字符），选区高亮就有几万像素宽：Sentry 里 4 次（LISTARY-5GS、5H4），起点越界和宽度越界都有。

改动（`LISTARY PATCH (far-geometry)`）：

- `fill_rectangle`（选区、光标、下划线）：几何保持 f32 交给 `process_rectangle_impl`，不再先转 16 位。
- `process_rectangle_impl` 的纯色填充和四条边框：先在 f32 下按原几何算好每一块，和裁剪区求交，再转 16 位。几何和裁剪区此时都是整数像素，所以在原来转得了的范围里，选中的像素和原来完全相同。
- 位置或尺寸不是有限数（NaN、无穷大）的矩形不画。
- 圆角矩形：离裁剪边的距离超过 8192 时按 8192 算。距离只在半径和边框宽度以内才影响画面，而绘制时会把窗口宽高加到这个 16 位数上。
- 线性渐变：`LinearGradientCommand` 的四个裁剪距离改成 32 位，`draw_linear_gradient` 里的乘积用 64/128 位算。原来超过 32767 时 16 位相加溢出（debug 下 panic）。
- `draw_path`：路径要按完整外框拟合和描边，不能只取可见部分。外框转不了 16 位时整条路径跳过。
- `fill_rectangle`、`draw_path` 的裁剪区改用 `try_cast`，转不了就跳过这一笔。

没改的：图片用 32 位坐标，本来不崩；字形位置用 `as i16` 饱和，超出的字形本来就在窗口外。SDF 字形 Listary 不用。

测试（`cargo test -p slint --no-default-features --features std,compat-1-2,renderer-software --test software_renderer_far_geometry --test software_renderer_far_geometry_pixels`）：

- `software_renderer_far_geometry.rs`：复现线上那几种选区、光标、超宽矩形，改前 7 条 panic。
- `software_renderer_far_geometry_pixels.rs`：每个超出范围的图元配一个只在窗口外不同的「近处双胞胎」，两者逐像素比较，整块缓冲和逐行渲染两条路径都比。覆盖纯色、边框、圆角、渐变的矩形在 1/1.25/1.5/2 倍下、三种旋转、正好在 ±32767/32768 边上；约 6 万像素长的选区（滑到末尾和不滑）；NaN 和无穷大不画；超出范围的 Path 跳过。改前 6 条全部 panic。旋转四分之一圈的竖直渐变允许差 1：角度经过 f32 的 `tan`（90° 或 270° 处很大但有限），渐变略斜，斜多少随长度变，双胞胎的颜色差 1。

升级：上游 master 仍是先转换后裁剪。上游不改就把这些处挑过去。

## 上游 PR 回移：软件渲染器画 `drop-shadow-*`（slint-ui/slint#13758）

v1.18.1 的软件渲染器不画 `drop-shadow-*`（`draw_box_shadow` 是空的），Listary 只能把每处阴影烘成九宫格图或叠几圈半透明圆角矩形来画。上游 PR [slint-ui/slint#13758](https://github.com/slint-ui/slint/pull/13758)「software: draw drop shadows」补上了它；这里把这个 PR 原样回移，**Slint 升到包含它的版本后删掉这一条**。

回移时 PR 还没有合进上游 master，取的是 PR 头 `a81fbf6b654c6be9a5fd5ac2ebb7bf6d33cbcc17` 的三个提交，用 `git cherry-pick -x` 原样挑到本分支（作者照旧，提交说明末尾记着上游提交号），本节另起一个提交：

| 上游提交 | 说明 |
|---|---|
| `05d8761f39dd8fc2b4a8931bc13cb8419b427439` | core: Move BoxShadowOptions out of the box-shadow-cache feature |
| `eb00c6627942a8fbe61a440158e95e3697c8e04a` | software: keep rounded rectangle anti-aliasing independent of the clip |
| `a81fbf6b654c6be9a5fd5ac2ebb7bf6d33cbcc17` | software: draw drop shadows |

三个提交打到本分支没有冲突。和上面路径裁剪那处补丁同在 `software/lib.rs`，但互不相干：那处只改 `RenderToBuffer` 画路径的两个函数，阴影走的是新加的 `process_box_shadow`。整段回移不逐处打 `LISTARY PATCH` 标记，以本节和那三个提交为准。

### 改了什么

- `internal/core`：`BoxShadowOptions` 从 `graphics/boxshadowcache.rs`（只在 `box-shadow-cache` 特性下编译）挪到新文件 `graphics/boxshadow.rs`，软件渲染器不开那个特性也能用；新加 `drop_shadow_bounding_rect`，`items.rs` 的 `BoxShadow::bounding_rect` 改用它，算法不变。femtovg 和 skia 只跟着改了引用路径，画法不变。
- `internal/renderers/software`：
  - `draw_functions.rs`：阴影按「圆角矩形与高斯核的卷积」解析地逐行算，不生成、不缓存图片。离圆角够远的行用预先算好的一维横向曲线；圆角附近的行按 σ/4 切成横条再合并。正态分布函数用 Q15 定点表。
  - `lib.rs`：`draw_box_shadow` 接上上面的画法，直接画屏、逐行、局部重绘三条路都走它；圆角矩形的抗锯齿不再随裁剪框变化（第二个提交）。
  - `scene.rs`：逐行模式的场景里多一种命令。
- 其余：文档里「软件渲染器不支持 `drop-shadow-*`」那句删掉；`builtin_elements.rs` 的属性说明改成「FemtoVG 和 Qt 不支持 spread」；新增截图测试 `drop-shadow-edge-cases` 及各渲染器的参考图，另两个阴影用例的参考图更新。

### 在软件渲染器里的实际含义

- `drop-shadow-blur`：σ = blur / 2（物理像素），实际再加上 1/12 像素² 的方差防锯齿：σ = √((blur/2)² + 1/12)。只画到形状外 blur（即 2σ）为止，和局部重绘用的 `bounding_rect` 一致。
- `drop-shadow-offset-x/-y`、`drop-shadow-color`（含透明度，元素 `opacity` 照常乘上去）：支持。
- `drop-shadow-spread`：支持，正负都可以；阴影的圆角是 `max(0, 圆角 + spread)`。
- `border-radius` 四角不同：支持。
- inset 阴影（`inner-shadow-*`）：不画。
- 一个元素只有一组 `drop-shadow-*`；多层阴影要用多个元素叠。阴影也画在元素自身底下，元素不透明时被盖住，半透明时会透出来。

### 测试

- PR 自带的软件渲染器单元测试（`draw_functions.rs`、`lib.rs` 里 `box_shadow`/`drop_shadow` 开头的）和截图测试 `tests/screenshots`（含 `drop-shadow-edge-cases`、`drop-shadow-per-corner-radius`、`drop-shadow-spread`）。
- 本分支原有的软件渲染器测试照常要过。

### 升级

升级 Slint 时看上游有没有合进 #13758（或同类改动）：合进了就去掉那三个提交和本节；没合进就从新版本标签拉新分支，把上表三个提交重新挑过去。挑过去要注意上游合并前可能还会改，以那时的 PR 为准。

## 软件渲染器的圆角裁剪

v1.18.1 的软件渲染器对 `clip: true` 只按外接矩形裁，`border-radius` 被忽略（`combine_clip` 里一句 `TODO: handle radius`），子元素会画出圆角之外。上游的问题单是 [slint-ui/slint#4176](https://github.com/slint-ui/slint/issues/4176)，还没有实现；维护者在 [slint-ui/slint#9098](https://github.com/slint-ui/slint/pull/9098) 里说过想要的做法（通用的圆角裁剪、只在圆角附近做额外计算、带抗锯齿），这里按这个思路自己补，代码标 `LISTARY PATCH`。

### 改了什么

- `rounded_clip.rs`（新文件）：`RoundedClip`（物理像素的矩形加四角半径）、某个像素被裁剪放行多少（0–255，按像素中心到圆角圆心的距离，1 像素宽的过渡），以及 `draw_line`：画一行时，圆角之间整段照常画，圆角外的像素跳过，曲线上的像素逐个画、再按放行比例和原来的像素混合。
- `lib.rs`：
  - `SceneBuilder` 带一串圆角裁剪，`RenderState` 记生效的个数；`combine_clip` 收到非零圆角时，按和矩形裁剪相同的物理矩形（同样取整、同样跟屏幕旋转）加一条，半径按 `draw_border_rectangle` 的方式夹到宽高的一半；`restore_state` 退回时去掉。处理器通过新方法 `ProcessScene::set_rounded_clips` 得知当前的裁剪。
  - 直接画屏：`RenderToBuffer::foreach_ranges` 有圆角裁剪时每行走 `draw_line`，所有图元（矩形、圆角矩形与边框、三种渐变、图片、文字、阴影）都经过这里；有圆角裁剪时不走 `TargetPixelBuffer` 的 `draw_rectangle`/`draw_texture` 加速钩子。
  - 逐行：`SceneItem` 多一个 `rounded_clip` 下标指向 `SceneVectors::rounded_clips`；逐行循环把原来的 `match` 包进闭包，有圆角裁剪的图元走 `draw_line`；「整段不透明遮挡」的优化不用于这些图元。
- `path.rs`：路径的覆盖率乘上裁剪的放行比例。逐行模式本来就不画路径，照旧。
- `draw_functions.rs`：`TargetPixel` 加 `mix`（按覆盖率在两个像素之间插值），自带的四种像素格式都实现；默认实现取较近的一个，没有抗锯齿。winit 的软件像素和分层窗像素（`sw.rs`、`ulw.rs`）也实现了。
- 嵌套的圆角裁剪各自的放行比例相乘。
- 文档里「不支持 `border-radius` 和 `clip: true` 一起用」那句删掉。

### 测试

- `rounded_clip.rs` 和 `lib.rs` 的单元测试：放行比例只在圆角处小于 255；有圆角裁剪时局部重绘只写脏区，脏区内和整画一致。
- 截图测试 `basic/clip-rounded`（矩形、带边框的圆角矩形、线性和径向渐变、图片、阴影、四角不同半径、嵌套）、`text/clip-rounded-text`、`path/clip-rounded-path`，覆盖直接画屏、四种旋转、逐行和逐行局部重绘；`basic/clip-border` 的参考图随之更新（第一块的内角现在是圆的）。
- 本分支原有的软件渲染器测试照常要过。

### 升级

升级 Slint 时看上游 #4176 有没有实现：实现了就去掉本节和对应提交，改用上游的；没实现就把这个提交挑到新分支。冲突多半在 `lib.rs` 的 `foreach_ranges` 和逐行循环，上游常改这两处。

## 软件渲染器的字形覆盖率修正

v1.18.1 的软件渲染器画字时，字形边缘的覆盖率（0–255）直接当透明度、在 sRGB 里线性混合，没有 gamma 或对比度修正。结果是深色底上的浅色字边缘偏暗，看起来细、发虚；浅色底上的深色字边缘也偏淡。DirectWrite 和 Skia 都在混合前修正覆盖率。这一修正只能在渲染器里做：应用一侧能改的只有颜色，提亮颜色会让字整体变亮（满覆盖的像素也变），而问题只在边缘的半覆盖像素上。代码标 `LISTARY PATCH (text-gamma)`。

### 改了什么

- `text_gamma.rs`（新文件）：按字色的亮度查一张 256 项的表，把覆盖率映射成修正后的覆盖率。表的算法照 Skia 的 `SkMaskGamma`（`build_gamma_table`）：背景取字色亮度的反面（字越亮、假定背景越暗），先按背景亮度把覆盖率抬高一点（对比度项，深色字主要靠它），再按 gamma 1.6 的混合换算回 sRGB 下的覆盖率（浅色字主要靠它）。参数 `GAMMA = 1.6`、`CONTRAST = 1.25`。
  - 覆盖率 0 和 255 不变：满覆盖的像素和字形轮廓都不动，只有抗锯齿边缘变实。字形位置、大小、栅格化方式（不做 hinting）都不变。
  - 表里每一项不小于原值：没有字因此变淡。各种字色加深的幅度相近（半覆盖 128 修正后约 150–165）：浅色字主要靠 gamma，深色字主要靠对比度，中灰两者都有。
  - 字色的透明度不进表，照旧在查表之后乘上去，所以半透明字（次要文字、快捷键提示）只是边缘变实，不会整体变亮。不在线性空间里混合，那样会把半透明灰字提亮很多。
  - 256 张表在第一次画字时一次算好、进程内共用（64 KB）；没有 `std` 的构建不修正。
- `target_pixel_buffer.rs`：`DrawTextureArgs` 加 `coverage_table`；`scene.rs`：`SceneTextureExtra` 加同名字段，`from_target_texture` 照抄。
- `lib.rs`：画字形的两处（`draw_text_paragraph` 和 `draw_glyph_run`）按字色取表；图片、缓存的位图、单色图标（编译器也把它们嵌成 alpha 图）和距离场字形填 `None`，照旧。
- `draw_functions.rs`：`draw_texture_line` 的 alpha 图分支先查表再乘透明度。

GL 渲染器（femtovg）没有改：它的字由 GPU 混合，Listary 在 Windows 上默认不用它。macOS 上产品窗口默认走 femtovg，不经过这里；只有无窗口的测试走软件渲染器。

### 测试

- `text_gamma.rs` 的单元测试：每种亮度的表两端不变、单调、不低于原值；白、浅灰、中灰、深色、黑五种字色半覆盖处都加深到相近的值；alpha 图像素混合时先读表再乘透明度（不透明和 0x92 各验一次），不设表时与原来一致；透明度不影响选哪张表。
- 本分支原有的软件渲染器测试照常要过。

### 升级

升级 Slint 时看上游软件渲染器是否已有文字 gamma／对比度修正：有就去掉本节和对应提交；没有就把提交挑到新分支。冲突多半在 `lib.rs` 构造 `DrawTextureArgs` 的几处（新增字段要补上）和 `draw_functions.rs` 的 alpha 图分支。

## 软件渲染器的字形 hint

v1.18.1 的软件渲染器不做 hinting，按轮廓原样画字，字号还截成整数像素（`draw_glyph_run` 里 `font_size.cast()`，125% 下 18.75 px 画成 18 px）。小字号的汉字横笔落在两行之间，画成两行灰，看起来发软。Listary 7.0（WPF）和系统的 DirectWrite 在这些字号上按字体的 TrueType 指令只在竖直方向对齐，竖直方向不抗锯齿，横笔落成一行实色。代码标 `LISTARY PATCH (glyph-hinting)`。

### 改了什么

- `fonts/vectorfont/hinting.rs`（新文件）：每个字体、每个字号怎么画，照 DirectWrite 推荐渲染模式的规则，从字体自己的 `gasp` 表定（在 Windows 11 上逐个字体、逐个字号用 `IDWriteFontFace3::GetRecommendedRenderingMode` 核对过）：
  - 字号先四舍五入成整数 ppem 再查表。
  - hint 不 hint、竖直方向抗锯齿不抗锯齿是两个独立的开关：「对称网格对齐」标志决定 hint，「对称平滑」标志决定竖直抗锯齿。「网格对齐」「灰度」两个老标志 DirectWrite 灰度模式不看（MV Boli 只有「网格对齐」，DirectWrite 照样不 hint）。
  - 两个开关都关的档要单独画：不 hint 的轮廓，竖直方向不抗锯齿（Roboto 8 ppem 以下、MV Boli 所有字号）。竖直方向按整数 ppem 缩放轮廓再取样，DirectWrite 的落行是这样（MV Boli 16.25 px 的数字落 11 行，跟 16 px 一样）。
  - 没有 `gasp` 或者只有 0 版 `gasp`（0 版没有对称标志，DirectWrite 当作没有，如 Agency FB、Arial Bold）：hint，20 ppem 以下竖直方向不抗锯齿，以上抗锯齿。
  - CFF 轮廓照旧不 hint：skrifa 的 CFF hint 和 DirectWrite 的结果对不上。
  - hint 用 skrifa 的 TrueType 解释器（`SmoothMode::Normal`，保留线性度量），只在竖直方向起作用。
  - 竖直不抗锯齿时，每行只在行中线取一次样，横向算精确覆盖率（曲线拆成直线，非零环绕规则）。坐标不是有限值或者位图超过 4M 像素时不画，交回 swash。竖直抗锯齿又 hint 时用 zeno 照常栅格化。
  - `HintingInstance` 按「字体、字号（1/64 px）、可变参数」缓存，最多 48 个，用得最少的先丢。
- `fonts/vectorfont.rs`：`VectorFont` 加 `with_exact_pixel_size`，字形按小数字号画。字号先取整到 1/64 px，字形缓存键、hint 缓存键、分档判断、hint 和 swash 都用这同一个字号，所以共用缓存的两个字号画出来一定一样。不 hint 又竖直抗锯齿的字号照旧走 swash。
- `lib.rs`：`draw_glyph_run` 把小数字号传给 `VectorFont`。

GL 渲染器（femtovg）没有改，macOS 产品窗口不受影响。

### 代价（Notebook 实测，release）

- 雅黑 UI 每个字第一次画：hint 的字号约 8–9.5 µs，原来不 hint 约 3.4–4.4 µs；不 hint 的字号（22 px 以上）不变。一屏约 200 个不同字形，按单字耗时算冷缓存多约 1 ms；缓存命中时不变。
- 内存：`HintingInstance` 最多 48 个。雅黑 UI 实测一个约 4.2 KiB，48 个约 0.2 MiB；别的字体一个多大取决于它的 hint 程序，没有逐个测。字形缓存照旧按约 1 MiB 像素封顶。

### 测试

- `hinting.rs` 的单元测试：分档规则（各种标志组合，含全关、只有灰度、只有网格对齐，以及没有 `gasp`、0 版 `gasp`）；同一缓存里共用 1/64 px 的两个字号不论谁先画结果都一样（Roboto 8.49/8.496/8.5，跨 8、9 ppem 两档）；两个开关都关的档画出整行；行中线栅格化的横向覆盖率、子像素偏移、落不到行中线的细笔、重叠轮廓和镂空、非有限坐标和超大位图交回 swash。
- `hinted_rows_match_directwrite_for_the_windows_ui_fonts`（只在 Windows 上跑，一种字体都没装就失败）：拿 `directwrite_rows.txt` 里 DirectWrite 灰度的落行数据对照，雅黑、正黑、游哥特、Malgun、Segoe UI、Nirmala、Arial Rounded、Segoe UI Variable 各 8 个字号，加上 Roboto、MV Boli、Agency FB、Javanese Text、Arial 各 10 个字号。竖直不抗锯齿的字号，9 ppem 起每种字体至少 97% 的字形落在同样的行上（实测 97.3–100%，最低的是 MV Boli，hint 过的字体都在 98.6% 以上）；9 ppem 以下至少 80%（实测 83–95%，见已知差异）。竖直抗锯齿的字号阈值上下的淡边可能落在不同一侧，至少 70%（实测 75–100%，最低的是雅黑 22.5 px，那里照旧不 hint）。逐像素的灰度值不同：DirectWrite 的灰度只有 8 级，横向也更虚。参考数据用自写的 DirectWrite 小程序生成。
- 本分支原有的软件渲染器测试照常要过。

### 已知差异

- 斜笔、撇捺在竖直方向不抗锯齿，有台阶，跟 7.0 和 DirectWrite 一样。
- 和 7.0 一样，小字号上有的汉字顶或底比同行差一行（hint 本身的结果，DirectWrite 也一样）。
- 9 ppem 以下竖直不抗锯齿时，落在两个行中线之间的细横笔会丢，DirectWrite 会补一行（TrueType 的漏点控制）。试过按列补点，跟 DirectWrite 对得反而更差，没有做。界面字号不到这么小。

### 升级

升级 Slint 时看上游软件渲染器是否已支持 hint：有就比较落行后再决定去留；没有就把提交挑到新分支。冲突多半在 `vectorfont.rs` 的 `render_vector_glyph` 和 `lib.rs` 的 `draw_glyph_run`。

## `i-slint-backend-winit` 1.18.1 —— 分叉说明（LANDING KIT）

> 本节原是 Listary 仓库 `app/vendor/i-slint-backend-winit/PATCH-NOTES.md`,那时补丁以整包副本放在 Listary 仓库里;搬到本分支后内容照旧,只改了与位置有关的句子。下文 `app/`、`docs/`、`tools/`、`cicd/` 开头的路径都在 Listary 仓库。

`internal/backends/winit` 在 v1.18.1 上(源码与 crates.io `i-slint-backend-winit` 1.18.1 发布包逐文件相同)加了下列功能补丁:
① **R-31 软件渲染兜底档的通知器包装与全量 present**(`renderer/sw.rs`,约 +270 行);
② **R-89 原子呈现(ULW)**(`renderer/ulw.rs` 整份新增 + 三个文件的接线);
③ ~~`D-13` 「系统拿走了指针捕获」的一位~~:260923 删除,产品改用 winit 的 `drag_window()`(§二点七);
④ ~~T-11 每窗指针坐标~~:升 1.18.1 时删除,上游已按窗保存指针状态(§六);
⑤ **T-14 行为修复**:隐藏窗停止重复请求重绘、按窗补充重绘(§七;动画期无障碍节流 260923 已删除);
⑥ **AccessKit 建树时销毁的组件不再留在内存里**(`accesskit.rs`、`event_loop.rs`,§八)。
版本号保持 `1.18.1`,通过 `[patch.crates-io]` 顶替 registry 版本,锁文件里不出现第二个
版本,`slint = "=1.18.1"` 的精确钉版纪律不受影响。与上游不同的文件只有 `lib.rs`、
`winitwindowadapter.rs`、`event_loop.rs`、`frame_throttle.rs`、`accesskit.rs`、`renderer/sw.rs`
和新增的 `renderer/ulw.rs`,共 45 行带 `LISTARY PATCH` 标记;其余文件与上游 1.18.1 逐字相同。

> **260907**:曾经还有第四处 —— 批 6 的 W1 二分诊断开关(`accesskit.rs`,260906),让
> accesskit 适配器可以不建。二分矩阵跑完后它已删除,`accesskit.rs` 回到上游 1.17.1 的
> 逐字副本;读数见
> `docs/specs/260731-rust-migration/260904-c22-auto-update/w1-status.md` §2。现在
> `accesskit.rs` 又带了 §八 的修复,不再是逐字副本。

> **260820 变更**:本分叉曾带第二处功能 —— 窗口级合成模式(`CompositionMode`,launcher
> 透明边距的 DirectComposition 逃生舱,+114 行/3 文件)。随渲染器默认切回 femtovg-GL
> (用户裁定 260820,决策记录
> `docs/specs/260731-rust-migration/260820-renderer-gl-default/spec.md`),该功能整体
> 退役:GL 经重定向位图天然支持 per-pixel alpha,逃生舱失去存在理由。三处补丁已还原为
> 上游逐字节原文;其完整机制、语义合同、实测表与成本裁定见 Listary 仓库原 PATCH-NOTES.md 在该日期前的 git 历史
> (`git log -- app/vendor/i-slint-backend-winit/PATCH-NOTES.md`)。
> ⚠ **随之而来的守护降级**:当年 `CompositionMode` 是公开新符号,分叉被静默丢掉时产品
> E0432 编译失败;现在仅剩的 sw.rs 补丁只改函数体/加内部类型,**没有编译级信号** ——
> 守护降为 `listary-ui/tests/slint_discipline.rs` 的两条内容级测试(锁文件 patch 生效 +
> sw.rs 关键内容还在),与 `i-slint-core` 分叉同级。补丁搬到本分支后,内容检查改为依赖来源检查,关键片段清单见 §三 第 4 步。

### 一、分叉清单（后续新增接点见§六、§七）

| 文件 | 加了什么 |
|---|---|
| `renderer/ulw.rs`(**整份新增**,约 +400 行,无上游对应物) | **R-89 原子呈现**:软渲直写常驻 DIB → **一次 `UpdateLayeredWindow`**(位置 + 尺寸 + 整幅像素)。详见 §二点五 |
| `lib.rs`(+约 130 行,`LISTARY PATCH (R-89)` 标记) | ① `WinitCompatibleRenderer::atomic_presentation()` 默认方法 + `AtomicPresentation` trait(几何截存 / 映射旁路的窄口);② `pub mod atomic_presentation`(产品面:`arm_next_window` / `take_next_window` / `show(window, activate)`,后者是逐次决定激活与否的显示,返回走了哪条路 `ShowPath`,260924);③ `create_window_adapter` 里**无条件**消费武装位并挑 `WinitUlwRenderer`;④ `atomic_renderer()` 工厂(非 Windows / 无软渲特性时交 `None`,武装窗降级为普通窗而不是开不出来) |
| `winitwindowadapter.rs`(+约 70 行,`LISTARY PATCH (R-89)` 标记) | ① `set_position` / `resize_window` 对 ULW 窗**不走 winit**(原点截存 + 同步派发 `Resized`);② `map_native_window` 单点(ULW 窗走裸 `ShowWindow`,绕开 winit 的 flag diff),每次显示先画一帧再映射;③ `ensure_window` 尾部的 `set_enabled_buttons` 对 ULW 窗跳过;④ `set_visibility` 的首帧预画对 ULW 窗跳过(交给 ②,免得首次显示画两遍);⑤ 原生窗建出来之前隐藏时,把待建属性的 `visible` 清回 false(普通窗经 `set_visible` 本来就清;不清的话建窗前「显示 → 隐藏」会在建窗时被重新映射);⑥ 已经呈现过的 ULW 窗收到系统报来的 `Resized` 时不改 Slint 记的尺寸,缩放变化时随新缩放一起改尺寸(`keeps_own_size`,见 §二点五「系统报来的尺寸」) |
| `renderer/sw.rs`(约 +270 行,均带 `LISTARY PATCH (R-31)` 标记) | ① `NotifyingSoftwareRenderer` 包装 + 生命周期触发点,转发 `RendererSealed` 的全部方法;② 全量 present(废弃按脏矩形 present) |
| `winitwindowadapter.rs`、`event_loop.rs`、`frame_throttle.rs`(`LISTARY PATCH (T-14)` 标记) | 隐藏窗停止帧定时器续跑、按窗补充重绘,见 §七 |
| `accesskit.rs`、`event_loop.rs`(`LISTARY PATCH (accesskit-destroyed-during-build)` 标记) | 建无障碍树前先实例化组件树;每次建树清掉已销毁组件的弱引用,见 §八。同一处先实例化也挡住了 AccessKit 重入崩溃(`LISTARY PATCH (accesskit-reentrant-getobject)` 注释 2 行),见 §八点一 |

原来还有一行 `LISTARY PATCH (260820)`:`handle_focus_change` 的 `borrow_mut()` 改 `try_borrow_mut`,
防无障碍客户端发 Focus 动作时重入 panic。1.18.1 上游已经是 `try_borrow_mut`,升版时删除。

### 一点五、许可

本分支是上游源码的 fork,**`LICENSES/` 三份原样保留、一字未改**
(`GPL-3.0-only` / `LicenseRef-Slint-Royalty-free-2.0` / `LicenseRef-Slint-Software-3.0`),
`Cargo.toml` 的 `license` 字段同样未动。分叉不改变本仓与 Slint 的许可关系:我们用的仍是
锁定版 1.18.1 的同一套授权,只是在 fork 分支上携带这些改动。

### 二、R-31 软件渲染器降级档(260818)

无 GPU 适配器环境(VDI/VM/驱动损坏)的降级档由
`listary_ui::backend::configure_renderer_backend` 的阶梯选中软件渲染器
(证据与裁定:`docs/specs/260731-rust-migration/reviews/260818-no-gpu-adapter-fallback.md`
及其后继执行记录;阶梯选 GL 之前的检查见 §七 的 GL 启动检查)。
上游软件渲染路径缺两件产品必需的东西,全部补在 `renderer/sw.rs`:

| 块 | 补什么 | 不补的后果(全部实测过) |
|---|---|---|
| `NotifyingSoftwareRenderer` 包装 + 生命周期触发点 | 上游 `SoftwareRenderer` 对 `set_rendering_notifier` 返回 `Unsupported`;包装器存下通知器,`WinitCompatibleRenderer` 各方法按 femtovg 同款时机点火(**Setup 必须延迟到首帧渲染**,`resume()` 时 adapter 还没存 winit 窗口,消费者拿不到 HWND —— 踩过)。包装器转发 `RendererSealed` 的**全部**方法:漏转发的方法会对包装器跑 trait 默认实现,不报编译错误(1.18.1 新增了 `text_layout_cache` 等四个文字方法,漏了只会静默改掉软件档的文字测量) | FSW chrome 子类/初始摆位、launcher 任务栏样式/揭幕全部静默失效:窗口拖不动、resize 不了 |
| 全量 present(废弃脏区包围盒 present) | Windows 在窗口 hide/show 时清空重定向面,softbuffer 的 DIB `age()` 看不见这件事;只 present 脏区 = 重现身的窗口只有最后一条脏带可见。上游为此加的 `occluded()` 钩子依赖 Windows 并不可靠发出的 `WindowEvent::Occluded`。渲染仍走脏区(贵的那步没变宽),只把 blit 变宽 —— 无条件成立,盖住一切重建/清空面的路径 | 召唤出的 launcher 只剩浮空文字(612×100 面板从未上屏) |

**全量 present 的去留等实测再定。** 升 1.18.1 时这一块照原样
重打。1.18.1 每次经 slint 显示窗口时整窗标脏,并按脏矩形 present(上游 #13500),经 slint
`show()` 的那条路可能已经不需要这一块。验法:去掉这一块编一版,在 slint-gl 的无 3D 虚拟机上,
或者在有 GPU 的机器上设 `LISTARY_X_RENDERER=software`,对召唤出的 launcher 做前后对比(面板整块
上屏;边距判据同下面的 `launcher-margin-probe.ps1`),FSW 隐藏后再显示也看一次。上游的整窗标脏只管
经 slint 显示的那条路;宿主自己映射或用 DWM cloak 藏起再亮出的窗(软件档的 launcher 就是常驻映射
加 cloak)要单独看。有脏带就保留。

**★ present 管线不变量:渲染器输出的预乘 alpha 一路原样到 DWM,任何环节不许改写
像素语义。** DWM 对重定向面按 per-pixel alpha 合成(实测:透明窗口根的边距正确透出
背后内容),这就是 `background: transparent` 窗口在软件渲染档零特判可用的机制根源。
本补丁的**首版**曾在 present 前把 alpha 强制成不透明(为掩盖上面那条脏区 present
缺陷)—— 结果 launcher 的透明边距变成一圈不透明黑边(用户走查当场发现)。教训:
呈现缺陷在 present 步修(上面那行),不许动渲染出来的像素。机器判据 =
`tools/repro/no-gpu-adapter/launcher-margin-probe.ps1`(品红背景板:边距 8/8 穿透、
面板非穿透)。

顺带的两条已知边界:① slint 软件渲染器**不画 `drop-shadow`**(源码零实现、静默忽略)
⇒ 软件档的 launcher 边距是全透明的「无阴影」形态,属用户预许的降级档,零改动自动成立;
② 通知器回调的 `GraphicsAPI` 实参是占位(`NativeOpenGL` + 恒 null 的
`get_proc_address`):软件路径没有原生图形 API 可交,产品两个消费者都忽略该参;
未来若有消费者要真 API,null 指针会让误用响亮失败而不是悄悄腐蚀。

**实测**(无 GPU 适配器的 VirtIO DOD guest,dev 档):FSW 可见+聚焦+静止
0.01–0.02 核(WARP 同场景 7.39–7.67 核);标题栏拖拽 (120,60) 精确落位;
launcher 召唤面板完整、边距全透明(margin-probe 8/8);FSW hide→重现身像素完整。

### 二点五、R-89 原子呈现(260830)

**为什么在这一层**:普通窗上屏是**两笔独立事务** —— `SetWindowPos` 改矩形(立即生效)+
present 换内容(下一拍生效);DWM 按自己的节拍采样,采到两笔之间就把旧画面钉在新原点上。
`UpdateLayeredWindow` 一次系统调用同时携带新位置、新尺寸、完整新画面,中间态**没有表达
方式**。这是契约级的解,不是概率级的压低(双仪器实测 0/1280 + 0/1600,灵敏度对照
116/120;提交 p50 0.67ms)。全部证据与二分矩阵:
`docs/reviews/260830-ulw-atomic-presentation.md`;落地记录:
`docs/specs/260731-rust-migration/260830-ulw-bars-landing/spec.md`。

**射程 = 两条搜索条与弹出菜单的各层窗**(文件管理器浮动搜索条、文件对话框搜索条;弹出菜单
与托盘菜单每一层一扇按内容大小的窗),由**建窗调用点**武装
(`listary_ui::window_birth::born_atomic_bar`),**不是**进程级选择 —— 260820 的
「femtovg-GL 进程级单权威」对其余每一扇窗一字未动,这几扇是**登记过的例外**。
菜单层窗走这条路是为了位置、尺寸与画面一次提交,以及透明像素让点击穿过;它们的显隐同样经
`atomic_presentation::show`,子层不激活。

| 块 | 补什么 | 不补的后果 |
|---|---|---|
| `renderer/ulw.rs` | 软渲直写常驻 top-down DIB(预乘 BGRA,`TargetPixel` 与 DIB 内存布局逐字节相同 ⇒ 应用侧零拷贝)+ 单次 `UpdateLayeredWindow` | 没有这个渲染器,武装位无处可去 —— 编译期就断(`atomic_renderer` 里引用它) |
| **两条位规则**(`enforce_atomic_window_style`,每次 present 无条件重申) | 「有 `WS_POPUP`、无 `WS_CAPTION`」+ `WS_EX_LAYERED` | R-87c 二分定谳:缺 POPUP 或带 CAPTION ⇒ **同样的 ULW 代码 40/40 全出血**。winit 两雷全踩(为 aero-snap 故意留 CAPTION;无 owner 就没有 POPUP),而 `WindowAttributes` 两条都表达不了 |
| **几何截存**(adapter 的 `set_position` / `resize_window` 分支) | 宿主照常调 slint 的 `set_position` / `set_size`,值被截存,随下一次 present 原子生效 | 少任何一维,那一维就退回 `SetWindowPos` —— 位移帧当场回来(逆变异自验实测) |
| **映射旁路**(`map_native_window` → `ulw.rs` 的 `set_mapped`) | ULW 窗的 show/hide 走裸 `ShowWindow`,不碰 winit 的 `WindowFlags` | winit 的 `apply_diff` 在**任何**flag 变化时都按自己的缓存重算两个 style 字(`winit-0.30.13 window_state.rs:390`)⇒ 把 CAPTION 加回来、把 LAYERED 拿掉,条当场变透明直到下一帧 |
| **逐次决定激活**(`atomic_presentation::show(window, activate)` → trait 的 `set_next_map_activation` → `set_mapped`,260924) | 照 WPF 的 `ShowActivated`:每次显示由调用方决定,`true` 用 `SW_SHOW`,`false` 用 `SW_SHOWNA`(WPF `Window.nCmdForShow` 常态下的同一对命令)。选择只留给这次 `show()` 引起的那一次映射,`set_mapped` 不论显示还是隐藏都先把它取走;没人选的显示(普通 `show()`)不激活。已显示着的窗不重新激活。winit 建窗时的 `attributes.active` 不再参与:这种窗建时隐藏,之后从不经 winit 映射 | 旧版照抄了一句错的 winit 规则(「首次不激活、之后都 `SW_SHOW`」),第二次召唤起条就抢走用户正在打字的对话框的前台;而 winit 的真实行为(出生不激活就永远 `SW_SHOWNOACTIVATE`)是锁定版本的实现细节,不是可依赖的逐次显示契约。C# 的条同时要「自动展示不抢焦点」和「召唤激活」,产品负责人裁定照 WPF 做逐次接口 |
| **映射前先画**(`map_native_window` 里,260924) | 每次显示(含首次,`set_visibility` 的首帧预画对 ULW 窗跳过)都先 `draw()` 一帧再 `ShowWindow`;重新显示时清掉 `set_visibility` 为 macOS 准备的尺寸补发:ULW 窗的尺寸由 `resize_window` 同步交给 Slint、记在 `self.size`,滞后的是原生窗(只在下一次 present 才变),补发读原生尺寸会把宿主隐藏期间设的新尺寸改回旧的。**刚建出来的窗第一次映射保留补发**(`first_frame_presented` 为假):那时原生窗才是对的,`self.size` 可能还是建窗前按缩放 1.0 记的,窗口元素却已按真实缩放布局;不补就按旧尺寸给缓冲区,150% 下软件渲染器断言失败(f63c91e5c 启动即 panic) | Windows 显示分层窗时不发绘制消息,映射出来的是**上一次**的 ULW 位图(旧位置、旧尺寸、旧内容),直到下一次属性变化才重画;上游只预画首帧 |
| **系统报来的尺寸**(`Resized` 分支,261002) | 已经呈现过的 ULW 窗收到 winit 的 `Resized` 时,不再用它改 Slint 记的尺寸(`self.size`)。这种窗的尺寸只由宿主的 `set_size` 决定,原生窗只在下一次呈现时跟上,系统报来的只是某次呈现的回声。还没呈现过的窗照旧接受,那时原生窗才是对的(见上一行)。最小化的零尺寸那段不受影响:`resize_event` 本来就不收零尺寸,ULW 渲染器的 `occluded` 是空操作。缩放变化改在 `ScaleFactorChanged` 分支里接:跨显示器时 winit 先发它、再用 `SetWindowPos` 按新缩放改原生窗,后者带来的 `Resized` 同样不收,所以在同一分支里随新缩放一起把 `self.size` 设成「原逻辑尺寸 × 新缩放」。缩放和尺寸同时改,中间不会有一帧按新缩放要画面、缓冲区却还是旧物理尺寸(缩放变大时软件渲染器会因缓冲区不够断言失败)。之后宿主照旧在下一回合按新缩放重设尺寸(两条条的 `reposition`、菜单的 `rescale`) | 建原生窗时系统会用 `WM_SIZE` 报出建窗时的尺寸(实测 1920×1117 和 663×57 两种);原生窗在事件循环准备等待时才建,winit 在处理回调期间收到的事件先存着,到下一轮的计时器跑完之后才投递。浮动条第一次召唤时,宿主在召唤事件里建出窗口、做暖机显示,下一轮的零延时计时器里收尾暖机、按结果设好尺寸再显示,这几条建窗时的尺寸要到这之后才投递,把刚设的尺寸盖回建窗时的尺寸;之后宿主算出的尺寸不变,不会再设,窗口一直停在旧尺寸,原点却是按新尺寸摆的。实测:资源管理器浮动条本次运行第一次召唤、结果在显示之前就发布时,条停在只有输入框的 57 px(150%),结果列表被裁掉,再输入一个字才恢复;带本补丁的包同样步骤 3 次都正常,未带补丁的包对照仍是 57 px |

映射这几块的行为测试:`listary-ui/tests/atomic_bar_visibility.rs`;「系统报来的尺寸」一行的回归测试是 `listary-ui/tests/atomic_bar_resize_after_show.rs`(在一个事件里建窗并第一次显示,零延时计时器里设新尺寸再显示,之后原生窗是新尺寸;再发一次缩放变化,尺寸随新缩放、和窗口元素的逻辑尺寸一致;只要桌面门)。ULW 窗经 Slint 显隐:普通
`show()` 和 `show(w, false)` 都不动前台;隐藏期间改的位置和颜色在 `show()` 返回时已经上屏;隐藏时
事件循环几乎不醒;程序显示之后真鼠标点输入框,条拿到前台和键盘焦点;显示后走前台阶梯拿到前台;
`show(w, true)` 拿到前台;原生窗建出来之前显示又隐藏的窗,建出来之后仍是隐藏的;产品的两条条照产品的暖机顺序第一次显示不崩、原生窗尺寸等于 Slint 记的尺寸(缩放不是 1.0 的机器才分得出修前修后)。开可见窗、抢前台到自己的测试窗、注入一次点击和一个字,走桌面门并另要
`LISTARY_TEST_ALLOW_FOREGROUND` 与 `LISTARY_TEST_ALLOW_INPUT`。

**★ 纪律:`SetLayeredWindowAttributes` 一次都不许调**。调一次会把窗口锁进常量 alpha /
色键模式,此后每一次 `UpdateLayeredWindow` 直接失败(err=87 实测),解锁要关掉再打开
`WS_EX_LAYERED`(而那会丢掉已提交的整幅画面)。站岗 =
`listary-ui/tests/slint_discipline.rs::the_product_never_calls_set_layered_window_attributes`。

**★ 纪律:软渲不画 `drop-shadow`**(上游 `i-slint-renderer-software` 1.17.1 和 1.18.1 的
`draw_box_shadow` 函数体都是字面 `{ // TODO }`)。两扇条今日无阴影(C# 原实现同样没有);
菜单层窗的阴影在边距里用几圈纯色圆角矩形手画。站岗 = `cicd/lint/lint-a-form-no-dropshadow.mjs`
(这几扇窗的 import 闭包)。

**已登记的缝**(不修,理由见落地 spec):ULW 窗上若有人调用 winit 的 flag 改动 API
(`set_decorations` / `set_window_level` / `set_resizable` …),两条位规则会被 winit 改写,
条会**透明到下一帧**。今天产品在 warm-up 之后不碰这些 API:显隐走 Slint 自己的
`show()`/`hide()`(260924 起;之前是宿主的裸 `ShowWindow`),经 `map_native_window` 映射,不经
winit 的 flag;`show()` 顺带的 `update_window_properties` 只在值变了时才改 flag,而条的装饰和
可缩放性出生后不变;尺寸与原点走截存。所以这条缝不可达;它是「将来有人加一句」的缝,而不是今天的缺陷。

### 二点七、`D-13` 系统拖窗之后的 `pressed` 位(260906 加,260923 删除)

原来的补丁:产品自己 `ReleaseCapture()` + `PostMessage(WM_NCLBUTTONDOWN, HTCAPTION)` 交给系统的
模态移动循环,循环吃掉那次 `WM_LBUTTONUP`,后端私有的 `EventLoopState::pressed` 于是恒 `true`,
`CursorLeft` 臂不再派 `MouseEvent::Exit` —— 拖过一次窗之后,指针移出再移回同一列不再报 hover。
分叉为此开了 `lib.rs` 的 `pub mod system_drag` 和 `event_loop.rs` 里消费它的三行。

删除理由:winit 0.30 的 `Window::drag_window()` 在 Windows 上做的是同一件事(`ReleaseCapture` +
投递 `WM_NCLBUTTONDOWN(HTCAPTION)`),而且它记得这次拖动是自己发起的,在 `WM_EXITSIZEMOVE`
里补投一条 `WM_LBUTTONUP`,`pressed` 照常清零。产品改走
`slint::winit_030::WinitWindowAccessor::with_winit_window(|w| w.drag_window())`
(`listary-ui/src/window_placement.rs::begin_system_drag`),分叉里不再需要任何东西。

行为守卫(不看分叉内部,升级后原样重跑):`listary-ui/tests/tutorial_window_system_drag.rs`,
真鼠标走「按下不动就松开 → 拖两次」,窗都要跟着光标走。原来还有第 ④ 步「移进列、一步跳出窗、
一步跳回同一列,两次都要报 hover」,但完全不拖时也只报一次,测不出拖动的
影响,260923 删掉;拖过之后的悬停交给最终集成版的人工走查。

应用侧的 `window_placement::finish_system_drag`(下一拍补一条 `PointerExited`,还回 core 的指针
抓取)保留:winit 补投的抬起理论上也会释放这个抓取,但真机上没有拿到去掉它之后控件仍点得动的
证据。无头岗哨
`tutorial_window_headless::a_drag_that_swallowed_its_release_still_leaves_the_buttons_clickable`
照旧守着。

### 三、升级税:slint 升版怎么重新打

1. 从新版的 Slint 标签拉新分支(例如 `listary/<新版>`),把本分支上的补丁提交搬过去;冲突就是上游改了补丁碰到的地方,逐处对照。
2. 要搬的是**三组**补丁(`git grep -n "LISTARY PATCH" -- internal/backends/winit` 一次列全):R-31、R-89,以及 §七 的
   T-14 行为修复。§二点七 的 `D-13` 和 §六 的 T-11 已删除,不再重打。
   `renderer/sw.rs` 的 R-31 一处,以及 R-89 的四处 —— `renderer/ulw.rs` 整份文件搬过来
   (它不依赖上游任何私有形状,只依赖 `WinitCompatibleRenderer` trait 与
   `i_slint_renderer_software::{SoftwareRenderer, TargetPixel, RepaintBufferType}`),
   加上 `lib.rs` / `winitwindowadapter.rs` 的接线。`sw.rs` 的 `NotifyingSoftwareRenderer` 是私有的:
   ulw 用上游的 `SoftwareRenderer`,两条搜索条不装通知器(260923 起;之前 ulw 复用这个包装,
   委托表漏方法的风险会带到所有机器上的搜索条)。
3. 确认上游没改这些地方的形状:`WinitCompatibleRenderer` trait 的方法集、`RendererSealed` 的方法集
   (包装器逐个转发,新方法漏转发不报错)、
   `SoftwareRenderer` 的构造 / `render(buffer, pixel_stride)` / `set_repaint_buffer_type`、
   `WinitCompatibleRenderer::resume` 的窗口存放时机、`WinitWindowAdapter` 的
   `set_position` / `resize_window` / `set_visibility` 三处落点、
   `Platform::create_window_adapter` 里「先跑属性钩子、再建渲染器」的顺序。
4. 搬完逐条核对下面的关键片段还在(以前由 Listary 仓库 `slint_discipline.rs` 读副本源码自动检查;
   补丁搬到本分支后,那边改为只检查依赖来自本分支)。再跑 Listary 仓库的
   `cargo test -p listary-ui --test slint_discipline`(依赖来源 + SLWA 零命中)和 §二、§二点五、§七 列的行为测试,
   在无 GPU 环境复跑 §二 的实测行,并按落地 spec 的验收段重跑一次真机位移帧计数(改后应为 0)。
   - `renderer/sw.rs`(R-31):`struct NotifyingSoftwareRenderer`、`RenderingState::RenderingSetup`、
     `RenderingState::AfterRendering`、无条件的 `.present()`(替换上游的 `present_with_damage`)。
   - `renderer/ulw.rs`(R-89):`WS_POPUP`、`WS_CAPTION`、`WS_EX_LAYERED`、`UpdateLayeredWindow`、
     `CreateDIBSection`、`self.next_map_activates.take()`、`SW_SHOWNA`。
   - `lib.rs`(R-89):`pub mod atomic_presentation`、`fn arm_next_window`、`pub fn show(`、
     `fn set_next_map_activation`、`fn atomic_renderer`。
   - `winitwindowadapter.rs`(R-89):`atomic.stash_origin`、`fn map_native_window`、
     `atomic presentation could not present before mapping`、`also arms a resize catch-up`、
     `if self.first_frame_presented.get() {`、`&& self.renderer.atomic_presentation().is_none()`、
     `hidden before its native window exists`。
5. **上游若自己补齐了等价能力**(软件渲染器支持 rendering notifier / 可靠的全量 present),
   本分叉立即作废:删目录、删 `[patch.crates-io]`。
6. **漏做基本不会编译失败**(见文首 260820 变更说明)。R-89 只把**一半**的自防护拿回来:
   `lib.rs` 的 `pub mod atomic_presentation` 有编译级信号(`listary-ui` 与
   `listary-app-host` 直接调它),但**渲染器本体、几何截存分支、两条位规则全在函数体里**
   —— 丢了以后两条搜索条编得过、跑得动、静默退回普通窗,闪烁回来。升版批必须按 tracker
   P0-14 的并批清单走,不能靠编译器提醒。

**1.17.1 → 1.18.1 的重打记录**。块都从 1.17.1 的分叉原样截取,
标记数逐文件对过:

- 上游把窗口事件处理从 `event_loop.rs` 搬进了 `winitwindowadapter.rs::dispatch_winit_window_event`,
  指针位置和按下状态成了每窗字段:T-11 整组删除(§六)。`handle_focus_change` 上游已改
  `try_borrow_mut`:260820 那一行删除。
- `WinitCompatibleRenderer::resume` 多了 `window_adapter_weak` 参数:`ulw.rs` 只跟签名,不用它。
- `RendererSealed` 新增 `text_layout_cache`、`text_content_widths`、`text_line_height`、
  `text_input_has_parley_layout`;`text_input_byte_offset_for_position` 改为同时返回光标亲和性,
  `text_input_cursor_rect_for_byte_offset` 多了亲和性参数。`sw.rs` 的包装器按新 trait 重写了转发。
- 上游 `sw.rs` 改成按脏矩形列表 present;R-31 的全量 present 照旧替换它,去留见 §二。
- `ensure_window` 尾部 winit #2990 的 `set_enabled_buttons` 绕法,上游没有改;R-89 的跳过条件照旧包住它。
- `frame_throttle.rs` 上游一字未改;`Window::has_active_animations()` 仍读全局标志:T-14 两处照搬。
- 上游新增 `renderer/femtovg/glprobe.rs`,见 §七 的 GL 启动检查。

### 四、追溯网(260820 更新)

一个后来者无论从哪条线索进来,都应在一跳内到达本节。现有入口(改动软件渲染兜底
行为时若新增入口,必须同样指回这里):

| 入口 | 指向 |
|---|---|
| `AGENTS.md` 呈现栈单一权威条目 | 分叉清单 + 本节 + P0-14 升版批 |
| `listary_ui::backend::configure_renderer_backend` 文档块(R-31 阶梯) | 本节 |
| `listary-ui/tests/slint_discipline.rs` 的依赖来源检查 | 分叉位置见 `app/upstream-forks.md`,重打配方 = 本节 §三 |
| `listary_ui::window_birth::born_atomic_bar`(搜索条与菜单层窗的出生式) | 本节 §二点五 |
| `cicd/lint/lint-a-form-no-dropshadow.mjs`(gate 第 11 检) | 本节 §二点五 的 drop-shadow 纪律 |
| `app/Cargo.toml` 的 `[patch.crates-io]` | 注释指 `app/upstream-forks.md` |
| tracker `P0-14`(slint 升版并批)| 分叉重打配方 = 本节 §三 |
| 渲染器决策记录 `docs/specs/260731-rust-migration/260820-renderer-gl-default/spec.md` | 合成模式功能的退役记录 |
| 分叉内部改动 | 均带 `LISTARY PATCH` 注释,`grep -rn "LISTARY PATCH"` 一次列全 |

### 五、证据锚点

- R-31 裁定与无 GPU 实测:`docs/specs/260731-rust-migration/reviews/260818-no-gpu-adapter-fallback.md`
- 渲染器切换决策(合成模式退役的上级依据):
  `docs/specs/260731-rust-migration/260820-renderer-gl-default/spec.md`
- 已退役的合成模式功能全档:原 PATCH-NOTES.md 260820 前的 git 历史
- R-89 原子呈现的结论档案与二分矩阵:`docs/reviews/260830-ulw-atomic-presentation.md`
- R-89 落地记录(退役对照表、验收、登记清单):
  `docs/specs/260731-rust-migration/260830-ulw-bars-landing/spec.md`
- 探针:`tools/repro/no-gpu-adapter/launcher-margin-probe.ps1`、
  `tools/repro/resize-jitter/strict-drag-probe.ps1`

### 六、T-11 指针坐标的窗口归属(260908 加,升 1.18.1 时删除)

原来的补丁修的是 1.17.1 事件循环级共享的指针位置:A 移到 P、B 移到 Q、A 没有新的 CursorMoved 时,
A 上的按下/抬起/滚轮拿到的是 Q。分叉用 `pointer_event.rs` 把位置和事件转换挪进每个 adapter。

1.18.1 把窗口事件处理搬进了 `winitwindowadapter.rs::dispatch_winit_window_event`,`cursor_pos` 和
`pressed` 本身就是每个 `WinitWindowAdapter` 的字段(`winitwindowadapter.rs:460/463`),按下、抬起、
滚轮读的都是目标窗自己的位置。补丁整组删除:`pointer_event.rs`、模块声明、事件循环接线、adapter
字段都不再有;直接编译 `pointer_event.rs` 的 `listary-ui/tests/winit_pointer_coordinates.rs` 一并删除。

和原补丁的一处差别:上游在 CursorMoved 时按当时的 scale 换成逻辑坐标存下;原补丁存物理坐标,每次
事件按当时的 scale 换算。窗口 DPI 变了而指针没动,下一次按下在上游用的是旧 scale 换出的位置。产品
没有依赖这一点的用例,跨显示器拖窗后原地点击留给真机走查。原实现和测试见原 PATCH-NOTES.md 升 1.18.1 之前的
git 历史。


### 七、T-14 行为修复（260909–260910）

260923 删除了 T-14 的诊断观测：`render_observer.rs`、`render_observer_windows.rs`、Windows 消息钩子
以及 adapter、事件循环、帧节流、无障碍里的各处接点，连同 `i-slint-core` 的 `update_observer.rs`。
T-14 已于 260910 结案，这些观测只服务于那次调查，不改变任何行为；诊断代码不留在
出货分叉里。原说明见原 PATCH-NOTES.md 260923 之前的 git 历史。
下面几节是保留的行为修复。

#### 260909 隐藏窗停止重复请求重绘（C，260923 缩成最小版）

上游缺陷:窗隐藏之后再有人请求重绘,`TimerBasedFrameThrottle` 的定时器靠 `pending_redraw()` 决定
续不续跑;隐藏窗收不到绘制消息,`draw()` 不跑,待画标志永远清不掉,定时器就一直跑(T-14 实测隐藏的
预览窗 1144 秒里请求了 25.5 万次,中位间隔 4.5 ms;规格 `docs/specs/260731-rust-migration/260909-t14-hidden-redraw/spec.md`)。

现在的补丁只有 `frame_throttle.rs` 定时器回调里的一个条件:窗是 `Hidden` 就不再续跑(1.18.1 的
`frame_throttle.rs` 和 1.17.1 一字不差,缺陷还在)。隐藏期间的
请求最多让定时器再醒一次;再显示时 `set_visibility` 照上游清掉待画标志,系统的绘制消息照常重画。
不分平台:隐藏窗在哪里都不需要帧定时器。260909 的版本(`redraw_state.rs` 状态机、`timer_redraw.rs`、
adapter 各处 transition 接线,约 700 行)已换成这一版:260923 的补丁重审认为一个条件就修得住同一个缺陷,整套状态机的重打成本不值。

260924 补上 OS 层面的隐藏(F5):两条挂靠搜索条用 `ShowWindow(SW_HIDE)` 自己隐藏窗口,
slint 不知道,`visibility()` 还是 `Shown`,条件拦不住。收起时最后一次发布请求的重绘就让定时器按
刷新率一直跑下去(实测收起后主线程每秒唤醒 316 次,约 0.7% 一个核)。现在定时器回调还问一次 winit
的 `is_visible()`(Windows 上就是 `IsWindowVisible`),OS 已经隐藏了也停。待画标志不清:普通窗显示时
系统会发绘制消息,积压的那次重绘照样画出来。

同日两条条改用 Slint 自己的 `hide()`/`show()`:slint 知道它们隐藏了,
上面第一个条件就拦得住;显示时由 `map_native_window` 先画一帧再映射,激活与否逐次由
`atomic_presentation::show` 决定(见 §二点五)。OS 层这一问留着,兜住以后再有人绕过 slint 隐藏窗口。

行为测试:`cargo test -p listary-ui --test winit_redraw_scheduling`(真事件循环、两扇真窗,
公开接口观测:两扇都隐藏、再请求一次重绘之后 1 秒里事件循环几乎不醒,再显示照常重画;
请求重绘后立刻用 `ShowWindow(SW_HIDE)` 在 OS 层隐藏,同样几乎不醒,`SW_SHOWNA` 再显示后照常重画)。
会开可见窗,走桌面门。条经 Slint 显隐的行为(不抢前台、显示后第一帧就是新的、隐藏时几乎不醒、
点得进去、召唤拿得到前台)在 `listary-ui/tests/atomic_bar_visibility.rs`,另要
`LISTARY_TEST_ALLOW_FOREGROUND` 与 `LISTARY_TEST_ALLOW_INPUT`。上游 PR 候选:与下面的按窗补充重绘合成一个。


#### 260909 GL 启动检查（已移出分叉）

关了 3D 加速的虚拟机上 WGL 能建出上下文、`GL_VERSION` 却为空，femtovg 的 glow 读到空版本会 panic。原先在本分叉的 `build` 里用 winit 窗和 glutin 实测并精确清理原生资源；260923 起改由应用在选渲染器之前自己检查：在一扇同步销毁的普通 Win32 窗上走和本后端 `glcontext.rs` 相同的 glutin 步骤并读 `GL_VERSION`（`listary-platform/src/opengl_probe.rs`，由 `listary_ui::backend::configure_renderer_backend` 调用）。升级 Slint 时核对 `glcontext.rs` 的建上下文步骤有没有变，探测要跟着改。

1.18.1 把 `glcontext.rs` 里建好上下文之后查 `glCreateShader` 入口的那一步挪进了新文件 `renderer/femtovg/glprobe.rs`：`GlutinFemtoVGRenderer::new_suspended` 建窗之前先调 `opengl_2_available()`（只走 WGL，查 shader 入口），查不过就返回错误，后端选择失败，落进 `backend.rs` 的软件档。所以上游检查和应用检查的结论一致，不会出现报告 femtovg、实际是软件渲染的情况。建上下文的其余步骤没变。应用侧的探测保留：它还读 `GL_VERSION`，比上游严。分叉里的预检、清理和测试包全部删除，`glcontext.rs`、`femtovg.rs` 与 `Cargo.toml` 回到上游原样。回归测试 `listary-ui/tests/opengl_startup_fallback.rs`。

#### T14：动画期间合并无障碍 dirty 更新（260910，260923 删除）

260923 删掉了这层节流:`accesskit.rs` 回到上游 1.17.1 的逐字副本,`accesskit_throttle.rs`、
`animation_state.rs` 和 headless 测试 `winit_accesskit_throttle.rs` 一并删除。

依据是 Notebook 桌面上的对照(对照后批准删除)。两个 Debug 产品:
一个带节流;另一个把传给节流的动画判断换成假,每次 dirty 都照上游直接重建。用 T-14 的抽屉配方
(选项 → 命令页,开两次)逐帧抓屏,看每次打开抽屉的中间位置数和最长停顿:带节流 6–9 个、≤31 ms,
关掉 6–8 个、≤37 ms,都和 T-14 修好后的 95eb(7–9 个、≤63 ms)一致,离修之前的 57e6(1 个、
170–222 ms)很远。抓屏前用 UIA 查过点击目标,两边的 AccessKit 都已激活。抽屉停帧是下面的按窗
补充重绘修好的,和这层节流无关。

只覆盖了抽屉这一个场景,UIA 客户端也只是轻量查询;读屏软件(讲述人等)没测,留给最后的人工测试。
原实现和测试见原 PATCH-NOTES.md 260923 之前的 git 历史。升版不再重打。


### T14：按窗口补充重绘（260910，260923 缩成最小版）

上游 `Window::has_active_animations()` 读的是全局动画标志(core 里自带 TODO,1.18.1 仍在 `api.rs:792`),`about_to_wait` 于是给
**每一扇**窗都补一次重绘:选项抽屉动画期间启动器也被逐帧重绘,抢走绘制消息(T-14 停帧的主要嫌疑)。

B(按窗补充重绘):adapter 的 `animated_in_last_draw` 记下这一次 `draw()` 前后全局标志是否由假变真,
即这扇窗自己求值过动画;`about_to_wait` 的补充 `request_redraw` 还要满足这一位。draw 开始前标志已为真
的窗无法归属,靠 core 的动画属性依赖照常请求。现在是 `draw()` 外包一层 + `about_to_wait` 一个条件,
不再有 `observe_draw` / `animation_check` 这些辅助函数。

A(跨 tick 保留动画检查,`animation_state.rs`)只服务于上面的无障碍节流,260923 随节流一起删除。

行为测试:`cargo test -p listary-ui --test winit_redraw_scheduling`(教程窗的状态转圈在动时,静止的
字形探针窗 1 秒里几乎不画)。


## 八、AccessKit 建树时销毁的组件

**现象**:有 UI 自动化客户端(讲述人、部分输入法、截图或远控软件)激活了无障碍树时,文件搜索大窗每搜一次,
默认堆里多出几百对块(约 2.8 KB 配约 200 B),只在关窗时才还。关掉图标、换软件渲染都照样涨;Listary 一次实测
里大窗开着搜 500 次,进程私有内存从 746 MB 涨到 2224 MB。

**成因**(上游 1.18.1 的 `accesskit.rs` / `winitwindowadapter.rs` 原样如此):
- 建树在 `AccessKitAdapter` 被可变借用期间进行(`reload_tree` 的延迟回调、`InitialTreeRequested`)。
  建树开头调 `WindowInner::ensure_tree_instantiated()`,读几何时还会求值 `ListView` 的布局绑定;两者都会
  更新 repeater,当场销毁不再需要的行组件(例如行里的 `if !item.placeholder : cells` 和它里面的图标)。
- 组件销毁时走 `WinitWindowAdapter::unregister_item_tree`,它对 AccessKit 适配器只 `try_borrow_mut`;
  借不到就什么都不做。`components_by_id` 里这个组件的 `ItemTreeWeak` 于是留下来,而 `VRc` 只要还有弱引用
  就不释放那块内存 —— 组件本体一直占着,直到窗口的 adapter 销毁。子组件的 `parent` 弱引用又把父组件的
  内存一起留住,所以是成对的块。
- 上游 `register_item_tree` 的注释已经承认「建树时会新建组件」,销毁那一侧没有处理。

**修法**(每处都有 `LISTARY PATCH (accesskit-destroyed-during-build)` 标记,`accesskit.rs` 7 行、`event_loop.rs` 1 行):
1. `reload_tree` 的延迟回调在借用适配器**之前**先 `ensure_tree_instantiated()`(只在已向客户端发过树时做,
   没有客户端时行为不变);`event_loop.rs` 在处理 `InitialTreeRequested` 前同样先实例化。多数销毁因此发生在
   借用之外,`unregister_item_tree` 能正常清理。
2. `build_new_tree` 开头调 `NodeCollection::forget_destroyed_components`:`components_by_id` 里升级不了的
   弱引用连同 `component_ids`、编号一起清掉。布局绑定里的销毁挡不住,这一步保证遗留最多留到下一次建树。
   清掉后那块内存可能被新组件复用,而 `text_state`(文本输入框的文字节点状态)按组件指针存、没有活的
   `ItemTreeRef` 删不了单项。所以另记一张 `text_state_components`(哪些组件在 `text_state` 里有条目),
   只有被清掉的组件里有这种组件时才整张清 `text_state`(上游在客户端断开时也是整张清)。结果行里没有
   `TextInput`,平时清的都是行,搜索框的文字节点状态保持不动;这份状态要一直留着,节点编号才稳定,
   读屏软件看到的才是「节点更新」而不是「子树替换」。

**没覆盖的一处**:`text_state` 按 `TextInput` 自己所在的组件存,而那个组件(例如 `LineEdit` 里面的内部
`TextInput`)可能从来没有在 `components_by_id` 里分到编号。这样的组件如果恰好在建树期间销毁,
`forget_destroyed_components` 看不到它,它在 `text_state` 里的旧条目照样留着。这是上游原有的情况
(上游在同一处同样跳过 `component_destroyed`),本补丁没有让它变坏,也没有修它;不能说成「文本缓存的生命周期
问题都修好了」。

**为什么应用侧绕不开**:泄漏发生在后端内部的借用冲突里,触发条件是外部客户端激活无障碍树,与应用怎么写
界面无关。公开接口里没有关闭 AccessKit 适配器、也没有从外面清理 `components_by_id` 的入口;不开
`accessibility` 特性会让所有窗口失去无障碍支持,不可取。

**测试**:没有加自动测试。复现需要一个真实的 UIA 客户端去激活树(AccessKit 只在收到 `WM_GETOBJECT` 后才建树),
headless 测试里造不出来。验证靠实测:有 UIA 客户端时文件搜索大窗分批连续搜,每批后比较默认堆已分配字节和
行组件块的个数(Listary 仓库 `docs/specs/260731-rust-migration/260926-memory-review.md` 第 5.3 节有改前改后的数字和测法)。

**升级**:升版时先看上游是否已修:`unregister_item_tree` 在借用冲突时仍能清掉 `components_by_id` 的条目,
或建树改在借用之外实例化,就删掉这几处。

### 八点一、AccessKit 重入崩溃（同一处修法）

**现象**:有 UI 自动化客户端时,新窗口一打开程序就崩,panic 在 `accesskit_windows-0.34.0/src/subclass.rs:51`
「RefCell already borrowed」。Listary 7.1.0.37 里是首次启动向导,5 次 / 4 个用户,栈里都有输入法(TSF)的帧。

**成因**(上游 1.18.1 原样如此):
- 处理 `InitialTreeRequested` 时,AccessKit 的 `update_if_active` 借着自己的窗口状态(`SubclassImpl::state`)
  调 `build_new_tree`;建树开头的 `ensure_tree_instantiated()` 会跑还没跑的 change handler。
- 向导根上 `forward-focus` 的初始聚焦就是这样一个 handler:设焦点让 winit 打开输入法(`set_ime_allowed` →
  `ImmAssociateContextEx`),输入法在 `PeekMessage` 里把 `WM_GETOBJECT` 派回同一个窗口,AccessKit 的窗口过程
  再借一次同一个状态,panic。`WM_SETFOCUS`/`WM_KILLFOCUS` 派回来也一样。
- 只改 AccessKit 不行:它在 `update_if_active` 里本来就假定回调不会重入窗口过程,本分叉不另带 accesskit 的分叉。

**修法**:就是 §八 第 1 条「借用之前先实例化」。`event_loop.rs` 在处理 `InitialTreeRequested` 前、`reload_tree`
的延迟回调在借用前,都先 `ensure_tree_instantiated()`;待跑的 handler 在 AccessKit 借用之外跑完,建树时就没有
handler 可跑了。两处注释补了这个理由(`LISTARY PATCH (accesskit-reentrant-getobject)`),免得以后只按泄漏的
理由取舍时把它删掉。

**为什么应用侧绕不开**:触发条件是外部客户端在窗口初始化跑完之前请求无障碍树,应用无法控制它何时到来;
建树在后端内部借用里进行,公开接口没有在它之前跑 handler 的入口。把向导的初始聚焦挪走只能避开这一个窗口,
任何会在 handler 里设焦点的窗口都一样。

**测试**:`internal/backends/winit/tests/accesskit_reentrant_getobject.rs`(Windows,要 `accessibility` 特性,
开一个不抢前台的 100×100 窗口):
`cargo test -p i-slint-backend-winit --features accessibility,renderer-software --test accesskit_reentrant_getobject`。
测试给窗口发 `WM_GETOBJECT(UiaRootObjectId)`,AccessKit 就像有 UIA 客户端一样请求初始树,不需要真的客户端;
同一个回合改一个属性,让它的 change handler 待跑,handler 里再发一次 `WM_GETOBJECT`,代替输入法抽消息。
去掉 `event_loop.rs` 那处先实例化时,panic 与线上相同(`subclass.rs:51`,经 `build_new_tree` →
`ensure_tree_instantiated`);有它时通过。§八 说「headless 造不出 UIA 客户端」,这个办法也可以用来给泄漏写测试。

**升级**:同 §八。上游若改成在借用之外建树或实例化,这条测试照常要过。

## 九、最大化窗口换 DPI（`scale-without-resize`）

**现象**:最大化或全屏的窗口换到缩放不同的显示器、或改系统缩放后,窗口被还原成非最大化,尺寸变成
「旧逻辑尺寸 × 新缩放」;在这之前的一帧,软件渲染按旧逻辑尺寸 × 新缩放要画面,缓冲区还是旧物理尺寸
(以前直接断言崩溃,见软件渲染器一节的「窗口尺寸大过缓冲区」)。

**成因**(上游 1.18.1 原样如此):
- winit 处理 `WM_DPICHANGED` 时,对最大化或全屏的窗口不改尺寸(`allow_resize` 为假),之后不会有 `Resized`。
- 后端的 `ScaleFactorChanged` 分支只改缩放,窗口元素仍是旧逻辑尺寸(上游那里是一句 TODO)。
- 下一次同步窗口属性时,`update_window_properties` 把这个旧逻辑尺寸当成窗口应有的尺寸,
  `resize_window` → winit `request_inner_size`,原生窗口被改大小,winit 顺带去掉最大化。

**修法**(`winitwindowadapter.rs` 的 `ScaleFactorChanged` 分支,`LISTARY PATCH (scale-without-resize)`):
窗口是最大化或全屏、又没有待交给 winit 的尺寸请求时,按不变的物理尺寸和新缩放立刻重算逻辑尺寸
(`resize_event`)。这正是 winit 不改尺寸的那几种情况;普通窗口照旧等 winit 报来的新尺寸,不多排一次版。
原子呈现窗走自己的 `keeps_own_size` 分支,不受影响。

**为什么应用侧绕不开**:缩放变化和随后的窗口属性同步都在后端内部、同一次事件处理里发生,应用在中间
没有入口;应用事后再设尺寸也晚了,窗口已经被还原。

**测试**:`internal/backends/winit/tests/dpi_change_keeps_size.rs`(Windows,开一个不抢前台的小窗):
`cargo test -p i-slint-backend-winit --features renderer-software --test dpi_change_keeps_size`。
先发一条 `SIZE_MAXIMIZED` 的 `WM_SIZE` 让 winit 记成最大化(窗口不真的最大化),再直接发 `WM_DPICHANGED`。
修前窗口被还原、尺寸从 150×120 变成 200×160;修后仍是最大化、150×120,逻辑尺寸 75×60。
真最大化窗口换 DPI 要在桌面上另验。

**升级**:上游若在 `ScaleFactorChanged` 里补上那句 TODO(没有 `Resized` 时按新缩放重设逻辑尺寸),就去掉这处。

## 十、原子呈现窗不进任务栏（`atomic-tool-window`）

**现象**:在桌面或资源管理器里打字弹出的浮动搜索条,第一次召唤后任务栏上留下一个标题为
「SearchBarWindow」的按钮,按 Esc 收起后也不消失。

**成因**:
- 任务栏在窗口**显示时**决定要不要给它按钮:没有 owner 的窗口会有按钮,除非它是 `WS_EX_TOOLWINDOW`;
  带 `WS_EX_APPWINDOW` 的一定有。显示之后再改样式,不会去掉已经有的按钮。
- winit 给每个没有 owner 的窗口加 `WS_EX_APPWINDOW`,而且每次改窗口标志都会按自己的缓存重算样式。
- 原子呈现窗(两种搜索条、弹出菜单的各层)第一次映射时还带着它;应用侧要等到第一次隐藏前才改样式,已经晚了。

**修法**(`renderer/ulw.rs` 的 `enforce_atomic_window_style`,`LISTARY PATCH (atomic-tool-window)`):
和 `WS_EX_LAYERED` 一起保证 `WS_EX_TOOLWINDOW`、去掉 `WS_EX_APPWINDOW`。这个函数在建表面时(第一次映射之前)
和每次呈现时都跑,所以原子呈现窗从出生起就是工具窗,winit 重算样式后也会在下一次呈现前改回来。

**为什么不只在应用侧改**:应用拿到原生窗口时 winit 已经建好了样式,而 winit 之后每次改窗口标志都会把
`WS_EX_APPWINDOW` 加回来;只有每次呈现前都执行的这一处能保证映射时样式是对的。应用侧另有一处配合修改
(先隐藏再改样式),两处一起生效。

**测试**:`internal/backends/winit/tests/atomic_window_off_taskbar.rs`(Windows,显示一个不抢前台的小条):
`cargo test -p i-slint-backend-winit --features renderer-software --test atomic_window_off_taskbar`。
线程内的 `WH_CALLWNDPROC` 钩子在窗口第一次收到 `WM_SHOWWINDOW` 时读扩展样式。修前是 `0xc0010`
(带 `WS_EX_APPWINDOW`、没有 `WS_EX_TOOLWINDOW`);修后是 `0x80090`。

**升级**:R-89 原子呈现是本分叉自己的功能,随 `ulw.rs` 一起带过去。

## 十一、按键事件带虚拟键码（`key-code`）

**现象**:文本框的 Ctrl+A/C/V/X/Z/Y 在大写锁定开着、或者俄语这类非拉丁布局下不起作用;
Ctrl+Insert、Shift+Insert 没有反应;有选区时 Shift+Delete 只删不剪。

**成因**:
- Slint 的按键事件只有 `text`,`TextInput` 按 `text == "v"` 这样比字符认标准编辑命令。
  大写锁定时 winit 给的是 "V",俄语布局给的是「м」,都对不上。
- Insert 键的 `text` 是私用区字符,在认标准命令之前就被「不插入特殊字符」那道过滤挡掉了。
- Shift+Delete 被当成普通的向前删除。

**修法**(`LISTARY PATCH (key-code)`):
- `KeyEvent` 加公开字段 `windows-virtual-key`(Rust 里是 `windows_virtual_key`,`int`):
  当前布局下的 Windows 虚拟键码,0 表示没有。`text` 不变。
  文档写明它是推导值:winit 不公开消息里的虚拟键码,所以由扫描码加窗口线程的键盘布局换算
  (`MapVirtualKeyExW(MAPVK_VSC_TO_VK_EX)`),小键盘等少数键可能和 `WM_KEYDOWN` 的 `wParam` 不同。
  只有 Windows 上的 winit 后端填它;其他平台、其他后端(包括 Windows 上的 Qt)和公开的 `WindowEvent::KeyPressed` 都是 0。
- `InternalKeyEvent::shortcut()`:Ctrl+字母先看虚拟键码(VK_A…VK_Z),没有键码时才看 `text`,
  且不分大小写。修饰键要精确:只按 Ctrl(Ctrl+Shift+Z 除外),同时按着 Alt 或 Win 都不算。
  新增 Ctrl+Insert 复制、Shift+Insert 粘贴(苹果平台不加)。
- `TextInput::key_event`:标准命令挪到 `key-pressed` 回调之后、光标移动和删除之前,Insert 不再被过滤挡掉。
  有选区时 Shift+Delete 剪切(只按 Shift,带 Ctrl、Alt 或 Win 不算);没有选区时照旧向前删一个字。只读框照旧只允许复制。
- AltGr 判断(`modifiers_for`)不动,仍按 `text` 和 `text_without_modifiers` 判断;
  判成 AltGr 时 Ctrl 被清掉,所以 AltGr+A 打出「ą」而不是全选;
  没判成 AltGr 的真 Ctrl+Alt 两位都留着,按上一条不算命令。

**测试**:`api/rs/slint/tests/text_input_key_code.rs`(Windows):
`cargo test -p slint --no-default-features --features std,renderer-software,compat-1-2 --test text_input_key_code`。
测试经 `WindowEvent::internal` 送入带键码的事件,走完整的 `TextInput` 分发:
大写锁定、俄语布局的六个编辑命令、两个 Insert 组合、有无选区的 Shift+Delete、只读框、
Ctrl+Shift+V、AltGr、真 Ctrl+Alt+V、Ctrl+Win+V、带 Alt 或 Ctrl+Alt 的 Shift+Delete,以及键码在按下、重复和松开时都能在 `key-pressed`/`key-released` 里读到。
只加字段、还没改匹配时,当时的 12 条里有 7 条失败;后补的 3 条修饰键用例(真 Ctrl+Alt、Ctrl+Win、带别的修饰键的 Shift+Delete)在收紧修饰键之前失败。现共 15 条。

**升级**:上游没有这个字段(物理键的讨论见 slint-ui/slint#4221、#12570)。上游给出等价的键身份以后,
把 `.slint` 和 Rust 里对 `windows-virtual-key` 的引用迁过去,再去掉这处分叉;在那之前随版本挑过去。
