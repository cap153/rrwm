# 默认列出所有可用指令
default:
    @just --list

# -------------------------------------------------------------
# 全局代理配置（终端无环境变量时的保底地址；可直接在此修改）
# -------------------------------------------------------------
MANUAL_PROXY := "http://127.0.0.1:7897"

# -------------------------------------------------------------
# 1. 本地快速构建 (glibc，适合日常开发测试)
# -------------------------------------------------------------

# 本地 Debug 编译（自动探测代理，测不通自动直连）
dev proxy="":
    @just _run_local_cargo "{{proxy}}" build

# 本地 Release 编译（自动探测代理，测不通自动直连）
build proxy="":
    @just _run_local_cargo "{{proxy}}" build --release

# -------------------------------------------------------------
# 2. Musl 全静态打包 (基于 Docker Alpine，可在任意 Linux 运行)
# -------------------------------------------------------------

# 使用 Alpine 容器编译全静态 musl 二进制（自动探测代理，测不通自动直连）
build-musl proxy="":
    #!/usr/bin/env bash
    set -euo pipefail

    PROXY=$(just _get_proxy "{{proxy}}")
    DOCKER_PROXY_ARGS=()
    DOCKER_BUILD_ARGS=()

    if [ -n "$PROXY" ]; then
        echo -e "\033[1;32m==> [OK]\033[0m 代理 [\033[1;36m$PROXY\033[0m] 连通正常，已开启代理加速..."
        DOCKER_PROXY_ARGS=(
            -e "http_proxy=$PROXY"
            -e "https_proxy=$PROXY"
            -e "all_proxy=$PROXY"
            -e "HTTP_PROXY=$PROXY"
            -e "HTTPS_PROXY=$PROXY"
            -e "ALL_PROXY=$PROXY"
        )
        DOCKER_BUILD_ARGS=(
            --network=host
            --build-arg "http_proxy=$PROXY"
            --build-arg "https_proxy=$PROXY"
            --build-arg "HTTP_PROXY=$PROXY"
            --build-arg "HTTPS_PROXY=$PROXY"
        )
    else
        echo -e "\033[1;33m==> [WARN]\033[0m 代理未开启或无法连通，自动降级为直连模式..."
    fi

    # 1. 检查本地是否已有固化的 rrwm-builder 镜像；若没有，通过管道创建（注意 build 使用 --network=host）
    if ! docker image inspect rrwm-builder:latest >/dev/null 2>&1; then
        echo -e "\033[1;35m==>\033[0m \033[1m首次运行：正在固化底层编译环境至本地镜像 rrwm-builder (仅需一次)...\033[0m"
        printf "FROM alpine:latest\nRUN apk add --no-cache curl gcc musl-dev libxkbcommon-dev libxkbcommon-static pkgconf\n" | docker build "${DOCKER_BUILD_ARGS[@]}" -t rrwm-builder:latest -
    fi

    # 2. 直接使用装好全部 C 依赖的镜像，0 秒开箱即用
    echo -e "\033[1;34m==>\033[0m \033[1m启动构建容器进行 musl 静态编译...\033[0m"
    docker run --rm -it \
      --net=host \
      -v "$(pwd)":/volume \
      -w /volume \
      -v rrwm-cargo-cache:/root/.cargo \
      -v rrwm-rustup-cache:/root/.rustup \
      "${DOCKER_PROXY_ARGS[@]}" \
      rrwm-builder:latest sh -c "
        [ -f /root/.cargo/env ] && source /root/.cargo/env || true && \
        if ! cargo --version >/dev/null 2>&1; then \
            echo -e '\033[1;35m==>\033[0m \033[1m首次安装/修复 Rust 极简核心 (仅 3 组件)...\033[0m'; \
            rm -rf /root/.rustup/* /root/.cargo/bin /root/.cargo/env 2>/dev/null || true; \
            curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal --no-modify-path; \
            source /root/.cargo/env; \
        fi && \
        echo -e '\033[1;32m==> [OK]\033[0m \033[1mRust 工具链就绪: '\$(cargo --version)'\033[0m' && \
        cargo build --release --target x86_64-unknown-linux-musl
      "
    echo -e "\033[1;32m==> 编译完成！\033[0m产物路径: \033[1;36mtarget/x86_64-unknown-linux-musl/release/rrwm\033[0m"

# 检查生成的 musl 二进制文件是否为全静态（无动态依赖）
check-musl:
    #!/usr/bin/env bash
    BIN="target/x86_64-unknown-linux-musl/release/rrwm"
    if [ ! -f "$BIN" ]; then
        echo -e "\033[1;31m==> [ERROR]\033[0m 未找到 $BIN，请先执行 'just build-musl'"
        exit 1
    fi
    echo -e "\033[1;34m==>\033[0m \033[1m文件属性:\033[0m"
    file "$BIN"
    echo ""
    echo -e "\033[1;34m==>\033[0m \033[1m动态依赖检查 (期望显示 'not a dynamic executable'):\033[0m"
    ldd "$BIN" || true

# -------------------------------------------------------------
# 3. 安装指令
# -------------------------------------------------------------

# 将 musl 静态二进制安装到 ~/.local/bin/rrwm
install-musl proxy="":
    #!/usr/bin/env bash
    set -euo pipefail
    BIN="target/x86_64-unknown-linux-musl/release/rrwm"
    if [ ! -f "$BIN" ]; then
        echo -e "\033[1;33m==> [INFO]\033[0m 未找到 musl 产物，正在先执行 build-musl..."
        just build-musl "{{proxy}}"
    fi
    mkdir -p "$HOME/.local/bin"
    cp -v "$BIN" "$HOME/.local/bin/rrwm"
    chmod +x "$HOME/.local/bin/rrwm"
    echo -e "\033[1;32m==> 已成功安装到\033[0m \033[1;36m$HOME/.local/bin/rrwm\033[0m"

# 将本地 release 二进制安装到 ~/.local/bin/rrwm
install proxy="":
    #!/usr/bin/env bash
    set -euo pipefail
    BIN="target/release/rrwm"
    if [ ! -f "$BIN" ]; then
        just build "{{proxy}}"
    fi
    mkdir -p "$HOME/.local/bin"
    cp -v "$BIN" "$HOME/.local/bin/rrwm"
    chmod +x "$HOME/.local/bin/rrwm"
    echo -e "\033[1;32m==> 已成功安装到\033[0m \033[1;36m$HOME/.local/bin/rrwm\033[0m"

# -------------------------------------------------------------
# 4. 清理
# -------------------------------------------------------------

# 清理 target 产物
clean:
    cargo clean

# 清理 Docker 编译缓存 Volume 与专属镜像（释放磁盘空间）
clean-docker-cache:
    docker volume rm -f rrwm-cargo-cache rrwm-rustup-cache
    docker rmi -f rrwm-builder:latest 2>/dev/null || true

# -------------------------------------------------------------
# 5. 私有辅助指令（以 _ 开头，不显示在 just 列表中）
# -------------------------------------------------------------

# 核心代理探测器：输出有效代理地址，若不通则输出空字符
_get_proxy proxy="":
    #!/usr/bin/env bash
    set -euo pipefail
    ENV_PROXY="${http_proxy:-${HTTP_PROXY:-${all_proxy:-${ALL_PROXY:-${https_proxy:-${HTTPS_PROXY:-}}}}}}"
    CLI_PROXY="{{proxy}}"
    PROXY="${CLI_PROXY:-${ENV_PROXY:-{{MANUAL_PROXY}}}}"

    if [ -n "$PROXY" ] && curl -sI --connect-timeout 2 -x "$PROXY" https://static.rust-lang.org >/dev/null 2>&1; then
        echo "$PROXY"
    else
        echo ""
    fi

# 本地命令代理包装器：连通则注入代理，不通则清理残留死代理
_run_local_cargo proxy *args:
    #!/usr/bin/env bash
    set -euo pipefail
    PROXY=$(just _get_proxy "{{proxy}}")

    if [ -n "$PROXY" ]; then
        echo -e "\033[1;32m==> [OK]\033[0m 代理 [\033[1;36m$PROXY\033[0m] 连通正常，本地编译启用代理..."
        export http_proxy="$PROXY" https_proxy="$PROXY" all_proxy="$PROXY" \
               HTTP_PROXY="$PROXY" HTTPS_PROXY="$PROXY" ALL_PROXY="$PROXY"
    else
        echo -e "\033[1;33m==> [WARN]\033[0m 代理未开启或无法连通，本地编译自动直连..."
        unset http_proxy https_proxy all_proxy HTTP_PROXY HTTPS_PROXY ALL_PROXY 2>/dev/null || true
    fi

    exec cargo "$@"
