# Listary 对 Slint 1.18.1 的补丁

本分支 `listary/1.18.1` 从 Slint 的 `v1.18.1` 标签拉出，只放 Listary 需要的补丁。Listary 仓库的 `app/Cargo.toml` 用 `[patch.crates-io]` 从这个分支取被补丁的包，版本号不变。每处改动都有 `LISTARY PATCH` 注释。

## `i-slint-core`（`internal/core`）

`internal/core` 相对 v1.18.1 只改了一处：文字排版缓存清扫时，保留缓存条目和它的依赖追踪器，只丢掉排好的字形。v1.18.1 的 `internal/core` 与 crates.io 上 `i-slint-core` 1.18.1 发布包的源码逐文件相同（差别只在发布时规范化的 `Cargo.toml`、cargo 生成的元数据和展开的许可证符号链接）。

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

与 v1.18.1 的差别：`git diff v1.18.1 -- internal/core` 应只列出这两个文件。

### 测试

- Listary 仓库 `app/crates/listary-ui/tests/text_layout_cache_sweep_keeps_repaint.rs`：最小窗口，一个固定大小的标题（最先画）加 256 行的 `for`。
  - 第一段：条目数恰好在第 4 轮最后一行超过上限，然后只改标题颜色、不改文字。于是标题自己的那次取用触发清扫，清空的正是它自己的条目，而追踪器是干净的。标题必须照常画出文字。清扫若放在检查之后，这里会画成空白。
  - 第二段：再换 10 轮后改 3 次标题文字，标题像素每次都要变。用原版 1.18.1 时第一次改文字就失败。
  - 第三段：再换 10 轮，让标题的条目被清空；先读一次标题的 `preferred-width`（布局查询，不是绘制），再改文字，标题像素要变。若把清空的条目改回「释放后重建」，这一段会失败，因为新追踪器登记给了布局绑定。
- Listary 仓库 `app/crates/listary-ui/tests/fsw_text_repaint_after_cache_sweep.rs`：真实的文件搜索窗，结果集换 30 轮后改查询和状态文字。用原版 1.18.1 时标题停在第一个新查询上。

两条测试都用 `MinimalSoftwareWindow` 加复用缓冲，按 Slint 要求才画帧，和桌面后端的局部重绘一致。它们放在 Listary 仓库里，随应用的测试一起跑。

### 升级

升级 Slint 时先看上游是否已修：`TextLayoutCache::sweep` 如果不再删除带追踪器的条目，或者改由调用方在缓存外读取文字，就可以去掉这份分叉，改回 registry 版本。上面两条测试照常要过。如果还要保留分叉，从新版本的标签拉新分支，再把这两处改动打上去。

## `i-slint-renderer-software`（`internal/renderers/software`）

`internal/renderers/software` 相对 v1.18.1 只改了一处：局部重绘时，`Path` 只画在本次要重画的区域里。

### 问题

设置窗口命令页、动作页打开「添加」下拉菜单后，鼠标在菜单项上移动，菜单下面那几行列表的线条图标会浮到菜单上面。

原因在 `lib.rs` 的 `RenderToBuffer`：局部重绘会重画所有和重画区域相交的元素。其他元素都经 `foreach_ranges` 只写重画区域里的像素；`process_filled_path` 和 `process_stroked_path` 却把 `Path` 画满它自己的裁剪框。鼠标移到某个菜单项上，只有那一项要重画，但它下面那行的图标与之相交，于是整个图标被重画，压在重画区域外、上一帧已经画好的菜单像素上。

### 改动

`lib.rs` 一个文件，`grep -n "LISTARY PATCH" internal/renderers/software/lib.rs` 可以列全：

- 新增 `dirty_clips`：把 `Path` 的裁剪框和重画区域的每个矩形分别取交集。
- `process_filled_path`、`process_stroked_path` 对每个交集各画一次。重画区域最多 3 个矩形，每次都按整个路径大小生成遮罩，所以一个路径最多多生成两次遮罩；界面里的线条图标都很小，代价可以忽略。

与 v1.18.1 的差别：`git diff v1.18.1 -- internal/renderers/software` 应只列出 `lib.rs`。

### 测试

- 本分支 `internal/renderers/software/lib.rs` 的 `a_path_draws_only_inside_the_dirty_region`：重画区域只有左上角一小块时，填充和描边的路径都不能写到区域外，区域内照常画出。用原版 1.18.1 时两种都会写满整个路径。
- Listary 仓库 `app/crates/listary-ui/tests/path_redraw_stays_in_dirty_region.rs`：最小窗口里一个 `Path` 图标被上层矩形盖住，只改上层一个小块的颜色后，图标不能透出来。

### 升级

升级 Slint 时先看上游的 `RenderToBuffer` 画路径时是否已按重画区域裁剪；已经裁剪的，就去掉这处补丁。上面两条测试照常要过。

## `i-slint-backend-winit` 1.18.1 —— 分叉说明（LANDING KIT）

> 本节原是 Listary 仓库 `app/vendor/i-slint-backend-winit/PATCH-NOTES.md`,那时补丁以整包副本放在 Listary 仓库里;搬到本分支后内容照旧,只改了与位置有关的句子。下文 `app/`、`docs/`、`tools/`、`cicd/` 开头的路径都在 Listary 仓库。

`internal/backends/winit` 在 v1.18.1 上(源码与 crates.io `i-slint-backend-winit` 1.18.1 发布包逐文件相同)加了下列功能补丁:
① **R-31 软件渲染兜底档的通知器包装与全量 present**(`renderer/sw.rs`,约 +270 行);
② **R-89 原子呈现(ULW)**(`renderer/ulw.rs` 整份新增 + 三个文件的接线);
③ ~~`D-13` 「系统拿走了指针捕获」的一位~~:260923 删除,产品改用 winit 的 `drag_window()`(§二点七);
④ ~~T-11 每窗指针坐标~~:升 1.18.1 时删除,上游已按窗保存指针状态(§六);
⑤ **T-14 行为修复**:隐藏窗停止重复请求重绘、按窗补充重绘(§七;动画期无障碍节流 260923 已删除)。
版本号保持 `1.18.1`,通过 `[patch.crates-io]` 顶替 registry 版本,锁文件里不出现第二个
版本,`slint = "=1.18.1"` 的精确钉版纪律不受影响。与上游不同的文件只有 `lib.rs`、
`winitwindowadapter.rs`、`event_loop.rs`、`frame_throttle.rs`、`renderer/sw.rs` 和新增的
`renderer/ulw.rs`,共 28 处 `LISTARY PATCH` 标记;其余文件与上游 1.18.1 逐字相同。

> **260907**:曾经还有第四处 —— 批 6 的 W1 二分诊断开关(`accesskit.rs`,260906),让
> accesskit 适配器可以不建。二分矩阵跑完后它已删除,`accesskit.rs` 回到上游 1.17.1 的
> 逐字副本;读数见
> `docs/specs/260731-rust-migration/260904-c22-auto-update/w1-status.md` §2。

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
| `winitwindowadapter.rs`(+约 70 行,`LISTARY PATCH (R-89)` 标记) | ① `set_position` / `resize_window` 对 ULW 窗**不走 winit**(原点截存 + 同步派发 `Resized`);② `map_native_window` 单点(ULW 窗走裸 `ShowWindow`,绕开 winit 的 flag diff),每次显示先画一帧再映射;③ `ensure_window` 尾部的 `set_enabled_buttons` 对 ULW 窗跳过;④ `set_visibility` 的首帧预画对 ULW 窗跳过(交给 ②,免得首次显示画两遍);⑤ 原生窗建出来之前隐藏时,把待建属性的 `visible` 清回 false(普通窗经 `set_visible` 本来就清;不清的话建窗前「显示 → 隐藏」会在建窗时被重新映射) |
| `renderer/sw.rs`(约 +270 行,均带 `LISTARY PATCH (R-31)` 标记) | ① `NotifyingSoftwareRenderer` 包装 + 生命周期触发点,转发 `RendererSealed` 的全部方法;② 全量 present(废弃按脏矩形 present) |
| `winitwindowadapter.rs`、`event_loop.rs`、`frame_throttle.rs`(`LISTARY PATCH (T-14)` 标记) | 隐藏窗停止帧定时器续跑、按窗补充重绘,见 §七 |

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

映射这几块的行为测试:`listary-ui/tests/atomic_bar_visibility.rs`。ULW 窗经 Slint 显隐:普通
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
