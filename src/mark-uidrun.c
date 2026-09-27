/* iso-cc mark 引擎会话根助手（票 18；阶段 A 择型 (b) file-cap 的产品化形态）。
 *
 * 编译（setup 期，uid 编译期钉定——阶段 A 登记：防 env 注入任意 uid）：
 *   cc -O2 -std=c11 -Wall -DMARK_UID=4210 -o uidrun mark-uidrun.c
 * 安装（rootful 步骤，setup 打印、用户逐条审计应用）：
 *   setcap cap_setuid,cap_setgid=ep <state>/mark/uidrun
 *
 * 与阶段 A PoC（/tmp/iso-cc-exp18/uidrun.c）的差异（阶段 B 生命周期收敛）：
 * 1. uid/gid 编译期常量（原 PoC 读 UIDRUN_UID env——任意 uid 注入面，登记为必须收敛项）；
 * 2. setuid 后立即 setgroups(0, NULL) 清空附加组（原 PoC 泄漏 wheel，登记为必须收敛项；
 *    file caps 在 execvp 目标时因目标无 capability xattr 而自然消亡，内核语义）；
 * 3. 常驻 reaper：fork + PR_SET_CHILD_SUBREAPER + 信号转发 + 同 uid 收编击杀。
 *    mark 会话树 = uid 4210，iso-cc 宿主进程（uid 1000）对其 kill/wait 全 EPERM
 *    （阶段 A 事故留档：http.server 需 sudo 杀）——树的生命周期必须由同 uid 的
 *    本助手结构性保证（R8/US18：会话死 = 树死，零 residue）：
 *      - cc（fork 子）自挂 PDEATHSIG→助手：助手死 ⇒ cc 死（内核信号，无竞态窗口外的存活）；
 *      - 助手死 ⇒ cc 的孤儿 reparent 到助手（subreaper）⇒ 助手同 uid SIGKILL 收编；
 *      - iso-cc 死 ⇒ 助手 PDEATHSIG（父侧 pre_exec 挂）⇒ 逐级传导。
 * 4. `--session-id <id>` 前缀 flag 仅用于 list/sweep 的 argv 可见性（/proc/cmdline
 *    全局可读；uid 4210 的 environ 对宿主 EACCES，env 标记键在 mark 引擎不可用）。
 *
 * 调用形态（session.rs 构造）：
 *   uidrun [--session-id <id>] -- CMD [args...]
 *   uidrun --check   （打印编译期 uid/gid，setup 幂等比对 + 冒烟 sanity；无副作用）
 *   uidrun --probe   （真实执行一次降权后打印 probe-ok——spawn 前的 file-cap 在位断言）
 * 退出码：2 用法；1 降权/fork 失败（launch failure，父侧 fail-loud）；127 exec 失败。
 */
#define _GNU_SOURCE
#include <errno.h>
#include <grp.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <dirent.h>
#include <unistd.h>

#ifndef MARK_UID
#error "MARK_UID 必须编译期钉定（-DMARK_UID=<n>）：防 env 注入任意 uid（票 18 阶段 A 登记）"
#endif

/* 可选：宿主 tzdb 根（setup 期探测 /etc/zoneinfo 存在时以
 * -DMARK_TZDIR="/etc/zoneinfo" 编译）。file-cap exec 触发内核 secure-exec
 * 过滤，TZDIR 一类路径型变量被剥除（NixOS 宿主实测：date 解析 TZ 回落
 * UTC）——降权后重建，会话子进程经 execve 继承。 */
#ifdef MARK_TZDIR
static void restore_tzdir(void) {
    setenv("TZDIR", MARK_TZDIR, 0);
}
#else
static void restore_tzdir(void) {}
#endif

static volatile sig_atomic_t g_child = 0;

static void forward_signal(int sig) {
    if (g_child > 0) {
        kill((pid_t)g_child, sig);
    }
}

/* /proc 单遍扫描：ppid == 自身 的存活进程（收养子女，subreaper 语义）SIGKILL。
 * 返回击杀数；调用方循环至 0（防击杀窗口内的新孤儿）。解析 /proc/<pid>/stat
 * 第 4 字段 ppid：comm 可含空格/括号 → 从最后一个 ')' 之后取字段。 */
static int kill_adopted(void) {
    int killed = 0;
    pid_t self = getpid();
    DIR *d = opendir("/proc");
    if (!d) {
        return 0;
    }
    struct dirent *e;
    while ((e = readdir(d)) != NULL) {
        const char *n = e->d_name;
        for (const char *p = n; *p; p++) {
            if (*p < '0' || *p > '9') {
                goto next;
            }
        }
        {
            char path[64];
            char buf[4096];
            snprintf(path, sizeof(path), "/proc/%s/stat", n);
            FILE *f = fopen(path, "r");
            if (!f) {
                goto next; /* 已退出 / 权限不可得（非子女） */
            }
            size_t len = fread(buf, 1, sizeof(buf) - 1, f);
            fclose(f);
            buf[len] = '\0';
            char *close = strrchr(buf, ')');
            if (!close) {
                goto next;
            }
            long ppid = -1;
            if (sscanf(close + 1, " %*c %ld", &ppid) != 1) {
                goto next;
            }
            if ((pid_t)ppid == self) {
                pid_t pid = (pid_t)strtol(n, NULL, 10);
                if (pid > 0 && kill(pid, SIGKILL) == 0) {
                    killed++;
                }
            }
        }
    next:;
    }
    closedir(d);
    return killed;
}

/* 降权（file caps 语义下合法）：setgid → setgroups 清空 → setuid。
 * 返回 0 = 成功；-1 = 失败（已 perror）。 */
static int drop_to_mark_uid(void) {
    if (setgid((gid_t)MARK_UID) == -1) {
        perror("iso-cc uidrun: setgid");
        return -1;
    }
    if (setgroups(0, NULL) == -1) {
        perror("iso-cc uidrun: setgroups");
        return -1;
    }
    if (setuid((uid_t)MARK_UID) == -1) {
        perror("iso-cc uidrun: setuid");
        return -1;
    }
    return 0;
}

int main(int argc, char **argv) {
    if (argc >= 2 && strcmp(argv[1], "--check") == 0) {
        printf("uid=%d gid=%d\n", (int)MARK_UID, (int)MARK_UID);
        return 0;
    }
    if (argc >= 2 && strcmp(argv[1], "--probe") == 0) {
        if (drop_to_mark_uid() == -1) {
            return 1;
        }
        printf("probe-ok uid=%d gid=%d\n", getuid(), getgid());
        return 0;
    }

    /* argv 解析：`--` 前为 flag（--session-id <id> 仅作 list/sweep argv 键，忽略），
     * `--` 后为真实命令。 */
    int cmd_at = -1;
    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "--") == 0) {
            cmd_at = i;
            break;
        }
    }
    if (cmd_at == -1 || cmd_at + 1 >= argc) {
        fprintf(stderr, "usage: uidrun [--session-id <id>] -- CMD [args...]\n");
        return 2;
    }

    if (drop_to_mark_uid() == -1) {
        return 1;
    }
    restore_tzdir();
    /* PDEATHSIG 重建（票 18 生命周期实测补丁）：带 capability 的 execve 会被内核
     * 清除 PR_SET_PDEATHSIG（prctl(2) MAN PAGE：privileged execve 即清除）——
     * 父侧 pre_exec 挂的「iso-cc 死 ⇒ 助手死」链在进入本进程时已失效。此处
     * getppid() 即原始父（iso-cc）——重挂 + 竞态对账（==1 = 父已死，自尽）。 */
    {
        pid_t orig_parent = getppid();
        prctl(PR_SET_PDEATHSIG, SIGKILL, 0, 0, 0);
        if (getppid() != orig_parent) {
            return 127; /* 父已死（挂链竞态），不留孤儿 */
        }
    }

    /* 常驻 reaper 装配（顺序：信号 → subreaper → fork）。 */
    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = forward_signal;
    sigaction(SIGINT, &sa, NULL);
    sigaction(SIGTERM, &sa, NULL);
    sigaction(SIGHUP, &sa, NULL);
    sigaction(SIGQUIT, &sa, NULL);
    prctl(PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0);

    pid_t pid = fork();
    if (pid == -1) {
        perror("iso-cc uidrun: fork");
        return 1;
    }
    if (pid == 0) {
        /* 子：助手死 ⇒ cc 死（R8 结构对账）。prctl 后 getppid==1 = 助手已先死
         * （fork/prctl 竞态），不落地 exec。file caps 在 exec 目标时消亡（目标
         * 无 capability xattr → permitted=0）。 */
        prctl(PR_SET_PDEATHSIG, SIGKILL, 0, 0, 0);
        if (getppid() == 1) {
            return 127;
        }
        execvp(argv[cmd_at + 1], argv + cmd_at + 1);
        perror("iso-cc uidrun: execvp");
        return 127;
    }
    g_child = (sig_atomic_t)pid;

    /* 常驻等待（WNOHANG 轮询）：cc 或收养子女退出 ⇒ 收编击杀 + 排干 + 以 cc
     * 退出态终止。轮询兼作孤儿检测（票 18 生命周期实测：file-cap exec +
     * setuid 双重清 PDEATHSIG，父死亡信号链不可依赖）——getppid()==1 =
     * iso-cc 已死且本进程被 init 收养 ⇒ 击杀整树后退出（R8：树死随会话死）。 */
    for (;;) {
        int st = 0;
        pid_t w = waitpid(-1, &st, WNOHANG);
        if (w == 0) {
            if (getppid() == 1) {
                /* 孤儿：父（iso-cc）死亡，pdeathsig 不可依赖 → 结构性自毁 */
                kill(pid, SIGKILL);
                for (int guard = 0; guard < 100 && kill_adopted() > 0; guard++) {
                }
                while (waitpid(-1, &st, WNOHANG) > 0) {
                }
                return 1;
            }
            usleep(200000);
            continue;
        }
        if (w == -1) {
            if (errno == EINTR) {
                continue;
            }
            return 1;
        }
        if (w != pid) {
            continue; /* 收养子女先死：waitpid 已收尸 */
        }
        /* 会话根退出：收编击杀循环至无子女，排干僵尸。 */
        for (int guard = 0; guard < 100 && kill_adopted() > 0; guard++) {
            /* kill_adopted 生效窗口极短；guard 防御性上限 */
        }
        while (wait(NULL) != -1) {
            /* 排干（含已被 SIGKILL 的收养子女） */
        }
        if (WIFSIGNALED(st)) {
            int sig = WTERMSIG(st);
            signal(sig, SIG_DFL);
            raise(sig); /* 助手以同信号死 → 父侧见诚实信号态 */
            return 128 + sig;
        }
        if (WIFEXITED(st)) {
            return WEXITSTATUS(st);
        }
    }
}
