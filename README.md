# rustui
RustUI 是一个原生、跨平台、GPU 加速的**保留模式**（retained mode）GUI 框架： 应用构建一棵 Widget 树并常驻内存，框架在事件驱动下按帧执行 `update → layout → paint`，将绘制命令交给 wgpu 渲染。
