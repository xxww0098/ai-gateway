# 三角色设计：`super_admin` / `admin` / `user`

一份与具体产品无关的身份模型。任何带「客户 + 运维 + 安装主人」的系统都可以直接用。

授权只看账号上的 **role 字段**。会话令牌里不要放角色当权威；每次请求从存储重读。

代数是文末这份 `role_core.rs`：无 I/O、无框架、无 crate 依赖。拷走即可。

---

## 1. 三个词

| 存库值 | 含义 | 是谁 |
| --- | --- | --- |
| `user` | 用户 | 客户。用产品本身，管自己的资源。 |
| `admin` | 管理员 | 日常运维。进管理面，管业务对象和客户账号。 |
| `super_admin` | 超级管理员 | 这套安装的主人。运维权 + **谁能当管理员**。 |

内部用语 **staff** = `admin | super_admin`。只出现在代码里，不入库。管理面入口认 staff，不认「必须恰好等于 `admin`」。

没有第四个值。`superadmin`、`root`、`owner`、`administrator` 都不是角色：

- **读**：当 `user`（不给权限）
- **写**：拒绝

大小写不敏感，首尾空白丢掉。合法字面量只有表里三行。

---

## 2. 权限切在哪

不要给 `admin` 和 `super_admin` 两套管理后台。日常运维同一套管理面。

切点只有 **身份权**：

- 建号时写成哪个角色
- 改别人的角色、状态、密码、删除

| 能力 | user | admin | super_admin |
| --- | --- | --- | --- |
| 用产品、管自己的资源 | 是 | 是 | 是 |
| 进管理面（业务配置、客户账号、日常运维） | 否 | 是 | 是 |
| 给任意账号充值 / 调额度（若产品有钱或配额） | 否 | 是 | 是 |
| 建号为 `user`；改客户的资料、状态、密码 | 否 | 是 | 是 |
| 建号或改角色为 `admin` / `super_admin` | 否 | **否** | 是 |
| 改 staff 的角色、状态、密码、删除 | 否 | **否** | 是（受「最后一个」约束） |

admin 在列表里可以看见 staff 账号，编辑 / 删除对那些行禁用。界面藏按钮不够，**服务端必须拒**。

公开注册永远是 `user`。唯一例外是引导路径（§4）。

---

## 3. 代数

所有身份变更走同一个函数，handlers 不准各自 `if role ==`。

```
authorize(actor, change, target, last_active_super_admin) -> Result<(), Deny>
authorize_update(actor, target, new_role, leaving_active, last_active_super_admin) -> Result<(), Deny>
```

`change`：

| Change | 含义 |
| --- | --- |
| `Create { role }` | 建号。`target` 无意义。 |
| `Assign { to }` | 改 `target` 的角色。 |
| `Edit` | 改密码、显示名、余额、配额等。不会摘掉 SuperAdmin。 |
| `Disable` / `Delete` | 停用，或删除。 |

三条硬规则：

1. **不能给自己以上的角色。** `admin` 写不出 `admin` 或 `super_admin`。`user` 什么都写不出。
2. **不能改同级或上级。** `admin` 碰不了任何 staff。`super_admin` 可以改 `admin` 和其他 `super_admin`。
3. **最后一个处于可用状态的 `super_admin` 不可降级、停用、删除。** 改自己的密码、资料仍然可以。自降级只有在还存在另一个可用的 `super_admin` 时才行。

「最后一个」由宿主在同一事务里加锁点数人数，把 `bool` 传进代数。两个 super_admin 同时降对方，不能都成功。

拒绝只有三种，代数不讲产品文案：

| Deny | 含义 |
| --- | --- |
| `CannotAssign` | 无权分配该角色 |
| `CannotMutate` | 无权改这个账号 |
| `LastSuperAdmin` | 不能拿掉最后一个主人 |

解析：

- `parse(raw)`：空或未知 → 错误（写入路径用这个）
- `from_stored(raw)`：空或未知 → `user`（读取路径用这个，永远不给 staff 权）

密码哈希、HTTP、SQL 留在宿主。

---

## 4. 第一个主人

部署配置指定 **第一个 `super_admin`**，不是 `admin`。只认服务端环境变量（或 YAML 同名字段），永不取请求体。

### `.env`

进程启动时读这两项。空字符串和没写是一回事。邮箱会 trim + 小写；密码原样保留（首尾空格也算进长度）。

```env
# 第一个主人的登录邮箱。没写 = 不引导，谁先注册都是 user。
BOOTSTRAP_ADMIN_EMAIL=you@example.com

# 可选。有则空库启动时立刻建号；没有则等这个邮箱自己注册后再提权。
# 长度必须是 8–72 字节。种子成功后立刻删掉这一行。
BOOTSTRAP_ADMIN_PASSWORD=change-me-now
```

三种合法写法：

```env
# A. 空库立刻能登录（推荐首次部署）
BOOTSTRAP_ADMIN_EMAIL=you@example.com
BOOTSTRAP_ADMIN_PASSWORD=change-me-now
```

```env
# B. 只锁定邮箱：这个人注册（或库里已有这个账号）时升成 super_admin
BOOTSTRAP_ADMIN_EMAIL=you@example.com
# BOOTSTRAP_ADMIN_PASSWORD=
```

```env
# C. 不引导。第一个注册者仍是 user。
# BOOTSTRAP_ADMIN_EMAIL=
# BOOTSTRAP_ADMIN_PASSWORD=
```

非法：只写密码、不写邮箱 → **拒绝启动**。密码短于 8 或长于 72 字节 → **拒绝启动**。

YAML 等价字段是 `auth.bootstrap_admin_email` / `auth.bootstrap_admin_password`。环境变量非空时覆盖 YAML。优先写 `.env`，不要把密码提交进仓库。

Docker Compose 把这两项原样传进容器。VPS 部署把 `deploy/env.vps.example` 拷成机器上的 `.env` 再填。

### 启动时发生什么

引导条件：**存储里还没有一个可用的 `super_admin`**。已经有 `admin` 不挡住——那是救砖，不是「第一个注册者当老板」。

| `.env` | 还没有 super_admin 时 |
| --- | --- |
| 两项都空 / 都注释 | 不引导 |
| 只有邮箱 | 该邮箱注册或已存在 → 写成 `super_admin` |
| 邮箱 + 密码 | 不存在则建号（哈希后写入，不要明文）；已存在则只改 role，**不改密码** |
| 只有密码 | 视为配错，**拒绝启动** |
| 已有可用的 super_admin | 完全 no-op：不建号、不提权、不改密 |

密码是**引导密钥**，不是常驻凭证：

- 种子成功后，`.env` 里那份被忽略
- 以后改 `.env` 不会改登录密码
- 登入后在产品里改密，再删掉 `BOOTSTRAP_ADMIN_PASSWORD`
- 不要打日志、不要进追踪字段

没配 `BOOTSTRAP_ADMIN_EMAIL` 时，任何注册者都不是候选。写反就变成「谁先打开注册页谁当主人」。

存量数据全是 `admin`、没有 `super_admin` 时：不要把所有 admin 自动升成主人。配了引导邮箱且该账号存在 → 把他写成 `super_admin`；没配 → 系统保持「有运维、无主人」，admin 照常干活，只是谁都不能再提权，直到补上 `.env` 再启动一次。

---

## 5. 不变量

测试钉这些性质，不要把实现里的字符串抄进断言。

- `admin` 建号或改角色写成 `admin` / `super_admin` → 拒绝，存储不变
- `admin` 改另一个 admin 的密码、状态、删除 → 拒绝
- `super_admin` 可以把 `user` 提成 `admin`
- 唯一可用的 `super_admin` 自降级 / 自删 / 停用 → 拒绝；改自己的密码 → 可以
- 两个 super_admin 时，其中一个可以降另一个
- 空库 + 邮箱密码 → 启动后该账号能登录且角色是 `super_admin`
- 已有 super_admin 后，再用配置邮箱注册 → 仍是 `user`
- 未配引导时，第一个注册者不是 staff
- 未知 role 读出来不是 staff；写入被拒

---

## 6. 宿主要做的事

- 把存储里的字符串变成 `Role`
- 在事务里计算 `last_active_super_admin`
- 把 `Deny` 映射成本产品的错误体
- 引导建号时的哈希与写入
- 管理面用 `is_staff()`，不要写 `role == "admin"`
- 界面可以镜像 `can_assign` / `can_mutate` 来藏按钮，但不是安全边界

---

## 7. 不要做

- 给 super_admin 单独一套管理路由。身份权只发生在账号管理。
- 用邮箱名单、JWT claim、配置文件里的列表做运行时授权。权威在账号的 role 字段。
- 每次启动用配置覆盖已有密码。
- 给角色起别名。
- 把账号明文写进数据库迁移。
- 「谁先注册谁当管理员」。
- 给种子主人发客户激励（注册赠金、试用额度等）。
- 为「以后可能有第四档」做泛型角色框架。三档就是三档。

---

## 8. `role_core.rs`

```rust
//! Three-rank RBAC algebra. No I/O, no framework, no crate dependencies.
//!
//! Copy this file into another project. Persistence, HTTP and password
//! hashing stay in the host app.
//!
//! Stored strings (the only values that belong in a `role` column):
//! `user` · `admin` · `super_admin`.
//!
//! # Rules
//!
//! * `super_admin` may assign any role and mutate any account.
//! * `admin` may create/mutate `user` only — never staff.
//! * `user` may not assign or mutate anyone.
//! * The last active `super_admin` cannot be demoted, disabled, or deleted.
//!
//! Call [`authorize`] / [`authorize_update`] from handlers. Do not re-derive
//! these rules at the call site.

use core::fmt;

/// The three ranks this algebra knows. Unknown stored strings are not staff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    User,
    Admin,
    SuperAdmin,
}

impl Role {
    pub const ALL: [Self; 3] = [Self::User, Self::Admin, Self::SuperAdmin];

    /// Parse a stored or requested role. Empty / unknown is an error — use
    /// [`Role::from_stored`] when reading a column that must never fail.
    pub fn parse(raw: &str) -> Result<Self, ParseError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "user" => Ok(Self::User),
            "admin" => Ok(Self::Admin),
            "super_admin" => Ok(Self::SuperAdmin),
            "" => Err(ParseError::Empty),
            _ => Err(ParseError::Unknown),
        }
    }

    /// Column → rank. Empty and unknown collapse to [`Role::User`] so they
    /// never pass a staff gate.
    #[must_use]
    pub fn from_stored(raw: &str) -> Self {
        match Self::parse(raw) {
            Ok(role) => role,
            Err(ParseError::Empty | ParseError::Unknown) => Self::User,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Admin => "admin",
            Self::SuperAdmin => "super_admin",
        }
    }

    #[must_use]
    pub const fn is_staff(self) -> bool {
        matches!(self, Self::Admin | Self::SuperAdmin)
    }

    #[must_use]
    pub const fn is_super_admin(self) -> bool {
        matches!(self, Self::SuperAdmin)
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why [`Role::parse`] rejected the input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    Empty,
    Unknown,
}

/// An identity change the algebra can allow or deny.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Create a new account with this role. `target` is ignored.
    Create { role: Role },
    /// Change `target`'s role to `to`.
    Assign { to: Role },
    /// Password, username, balance, concurrency — never strips SuperAdmin.
    Edit,
    /// Move an active account off `active`.
    Disable,
    Delete,
}

/// Why [`authorize`] refused the change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deny {
    CannotAssign,
    CannotMutate,
    LastSuperAdmin,
}

impl fmt::Display for Deny {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CannotAssign => f.write_str("cannot assign that role"),
            Self::CannotMutate => f.write_str("cannot mutate that account"),
            Self::LastSuperAdmin => f.write_str("cannot strip the last super_admin"),
        }
    }
}

/// Allow or deny one identity change.
///
/// `last_active_super_admin` is computed by the host (count of active
/// `super_admin` rows, ideally under `FOR UPDATE`). Pass `false` when `target`
/// is not a super_admin.
pub fn authorize(
    actor: Role,
    change: Change,
    target: Role,
    last_active_super_admin: bool,
) -> Result<(), Deny> {
    match change {
        Change::Create { role } => assign(actor, role),
        Change::Assign { to } => match mutate(actor, target) {
            Ok(()) => match assign(actor, to) {
                Ok(()) => protect_last(
                    target,
                    leaves_super_admin(target, to),
                    last_active_super_admin,
                ),
                err => err,
            },
            err => err,
        },
        Change::Edit => mutate(actor, target),
        Change::Disable | Change::Delete => match mutate(actor, target) {
            Ok(()) => protect_last(
                target,
                matches!(target, Role::SuperAdmin),
                last_active_super_admin,
            ),
            err => err,
        },
    }
}

/// One update that may mix a role change, a disable, and field edits.
pub fn authorize_update(
    actor: Role,
    target: Role,
    new_role: Role,
    leaving_active: bool,
    last_active_super_admin: bool,
) -> Result<(), Deny> {
    match authorize(actor, Change::Edit, target, last_active_super_admin) {
        Ok(()) => {}
        err => return err,
    }
    match new_role == target {
        true => {}
        false => match authorize(
            actor,
            Change::Assign { to: new_role },
            target,
            last_active_super_admin,
        ) {
            Ok(()) => {}
            err => return err,
        },
    }
    match leaving_active {
        true => authorize(actor, Change::Disable, target, last_active_super_admin),
        false => Ok(()),
    }
}

#[must_use]
pub fn can_assign(actor: Role, role: Role) -> bool {
    assign(actor, role).is_ok()
}

#[must_use]
pub fn can_mutate(actor: Role, target: Role) -> bool {
    mutate(actor, target).is_ok()
}

fn assign(actor: Role, role: Role) -> Result<(), Deny> {
    match (actor, role) {
        (Role::SuperAdmin, _) => Ok(()),
        (Role::Admin, Role::User) => Ok(()),
        (Role::Admin, Role::Admin | Role::SuperAdmin) => Err(Deny::CannotAssign),
        (Role::User, _) => Err(Deny::CannotAssign),
    }
}

fn mutate(actor: Role, target: Role) -> Result<(), Deny> {
    match (actor, target) {
        (Role::SuperAdmin, _) => Ok(()),
        (Role::Admin, Role::User) => Ok(()),
        (Role::Admin, Role::Admin | Role::SuperAdmin) => Err(Deny::CannotMutate),
        (Role::User, _) => Err(Deny::CannotMutate),
    }
}

const fn leaves_super_admin(from: Role, to: Role) -> bool {
    matches!((from, to), (Role::SuperAdmin, Role::User | Role::Admin))
}

fn protect_last(target: Role, would_leave: bool, is_last: bool) -> Result<(), Deny> {
    match (target, would_leave, is_last) {
        (Role::SuperAdmin, true, true) => Err(Deny::LastSuperAdmin),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_strings_round_trip() {
        for role in Role::ALL {
            assert_eq!(Role::parse(role.as_str()), Ok(role));
            assert_eq!(Role::from_stored(role.as_str()), role);
        }
    }

    #[test]
    fn parse_ignores_case_and_surrounding_space() {
        assert_eq!(Role::parse(" Super_Admin "), Ok(Role::SuperAdmin));
        assert_eq!(Role::parse("ADMIN"), Ok(Role::Admin));
    }

    #[test]
    fn unknown_and_empty_stored_values_are_not_staff() {
        for raw in ["", "   ", "superadmin", "root", "administrator"] {
            let role = Role::from_stored(raw);
            assert!(!role.is_staff(), "{raw:?} must not be staff");
            assert_eq!(role, Role::User);
        }
    }

    #[test]
    fn staff_is_admin_or_super_admin() {
        assert!(!Role::User.is_staff());
        assert!(Role::Admin.is_staff());
        assert!(Role::SuperAdmin.is_staff());
        assert!(Role::SuperAdmin.is_super_admin());
        assert!(!Role::Admin.is_super_admin());
    }

    #[test]
    fn admin_cannot_mint_staff() {
        for role in [Role::Admin, Role::SuperAdmin] {
            assert_eq!(
                authorize(Role::Admin, Change::Create { role }, Role::User, false),
                Err(Deny::CannotAssign),
            );
        }
        assert_eq!(
            authorize(
                Role::Admin,
                Change::Create { role: Role::User },
                Role::User,
                false
            ),
            Ok(())
        );
    }

    #[test]
    fn admin_cannot_mutate_staff() {
        for target in [Role::Admin, Role::SuperAdmin] {
            assert_eq!(
                authorize(Role::Admin, Change::Edit, target, false),
                Err(Deny::CannotMutate),
            );
            assert_eq!(
                authorize(Role::Admin, Change::Delete, target, false),
                Err(Deny::CannotMutate),
            );
        }
        assert_eq!(
            authorize(Role::Admin, Change::Edit, Role::User, false),
            Ok(())
        );
    }

    #[test]
    fn super_admin_can_appoint_admin() {
        assert_eq!(
            authorize(
                Role::SuperAdmin,
                Change::Create { role: Role::Admin },
                Role::User,
                false
            ),
            Ok(())
        );
        assert_eq!(
            authorize_update(Role::SuperAdmin, Role::User, Role::Admin, false, false),
            Ok(())
        );
    }

    #[test]
    fn last_super_admin_cannot_be_stripped() {
        assert_eq!(
            authorize_update(Role::SuperAdmin, Role::SuperAdmin, Role::Admin, false, true),
            Err(Deny::LastSuperAdmin),
        );
        assert_eq!(
            authorize(Role::SuperAdmin, Change::Disable, Role::SuperAdmin, true),
            Err(Deny::LastSuperAdmin),
        );
        assert_eq!(
            authorize(Role::SuperAdmin, Change::Delete, Role::SuperAdmin, true),
            Err(Deny::LastSuperAdmin),
        );
        assert_eq!(
            authorize(Role::SuperAdmin, Change::Edit, Role::SuperAdmin, true),
            Ok(()),
            "password / balance edits must still work on the last owner"
        );
    }

    #[test]
    fn a_peer_super_admin_can_demote_another() {
        assert_eq!(
            authorize_update(
                Role::SuperAdmin,
                Role::SuperAdmin,
                Role::Admin,
                false,
                false
            ),
            Ok(())
        );
    }

    #[test]
    fn user_cannot_assign_or_mutate() {
        assert_eq!(
            authorize(
                Role::User,
                Change::Create { role: Role::User },
                Role::User,
                false
            ),
            Err(Deny::CannotAssign)
        );
        assert_eq!(
            authorize(Role::User, Change::Edit, Role::User, false),
            Err(Deny::CannotMutate)
        );
    }
}
```
