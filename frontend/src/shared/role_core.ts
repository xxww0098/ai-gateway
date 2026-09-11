/** Mirrors crates/gw-role/src/role_core.rs. Staff = admin | super_admin. */

export const ROLES = ['user', 'admin', 'super_admin'] as const
export type Role = (typeof ROLES)[number]

export function parseRole(raw: string | undefined | null): Role | null {
  switch ((raw ?? '').trim().toLowerCase()) {
    case 'user':
      return 'user'
    case 'admin':
      return 'admin'
    case 'super_admin':
      return 'super_admin'
    default:
      return null
  }
}

export function isStaff(role: string | undefined | null): boolean {
  switch (parseRole(role)) {
    case 'admin':
    case 'super_admin':
      return true
    default:
      return false
  }
}

export function canAssign(actor: string | undefined | null, next: Role): boolean {
  switch (parseRole(actor)) {
    case 'super_admin':
      return true
    case 'admin':
      return next === 'user'
    default:
      return false
  }
}

export function canMutate(actor: string | undefined | null, target: string | undefined | null): boolean {
  switch (parseRole(actor)) {
    case 'super_admin':
      return true
    case 'admin':
      return parseRole(target) === 'user'
    default:
      return false
  }
}

export function assignableRoles(actor: string | undefined | null): Role[] {
  return ROLES.filter(role => canAssign(actor, role))
}

export function roleLabel(role: string | undefined | null): string {
  switch (parseRole(role)) {
    case 'super_admin':
      return '超级管理员'
    case 'admin':
      return '管理员'
    default:
      return '用户'
  }
}
