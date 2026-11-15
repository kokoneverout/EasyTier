// Dual-stack validation migrated from the legacy workspace. Keep node host bits;
// normalize only network routes. No DNS lookups or permissive numeric IPv4 forms.
function ipv6GroupsFromParts(parts: string[]): number[] | null {
  const groups: number[] = []
  for (const part of parts) {
    if (part.includes('.')) {
      if (!/^(?:0|[1-9]\d{0,2})(?:\.(?:0|[1-9]\d{0,2})){3}$/.test(part)) return null
      const v4 = part.split('.').map(Number)
      if (v4.some(n => n > 255)) return null
      groups.push((v4[0] << 8) | v4[1], (v4[2] << 8) | v4[3])
    } else {
      if (!/^[0-9a-fA-F]{1,4}$/.test(part)) return null
      groups.push(Number.parseInt(part, 16))
    }
  }
  return groups
}

export function parseIpv6Groups(addr: string): number[] | null {
  const s = addr.trim()
  if (!s.includes(':')) return null
  if (s.includes('.')) {
    const parts = s.split(':')
    if (!parts[parts.length - 1]?.includes('.') || parts.filter(p => p.includes('.')).length !== 1) return null
  }
  const count = (s.match(/::/g) ?? []).length
  if (count > 1) return null
  const sides = s.split('::')
  const head = ipv6GroupsFromParts(sides[0] ? sides[0].split(':') : [])
  const tail = ipv6GroupsFromParts(sides[1] ? sides[1].split(':') : [])
  if (!head || !tail) return null
  if (!count) return head.length === 8 ? head : null
  if (head.length + tail.length > 7) return null
  return [...head, ...new Array<number>(8 - head.length - tail.length).fill(0), ...tail]
}

export function isValidIpv6Cidr(value: string): boolean {
  const [address, prefix, extra] = value.trim().split('/')
  return extra === undefined && /^\d{1,3}$/.test(prefix ?? '')
    && Number(prefix) <= 128 && parseIpv6Groups(address) !== null
}

export function normalizeIpv6RouteCidr(value: string): string | null {
  if (!isValidIpv6Cidr(value)) return null
  const [address, prefix] = value.trim().split('/')
  const groups = parseIpv6Groups(address)!
  for (let i = 0; i < 8; i++) {
    const bits = Number(prefix) - i * 16
    if (bits <= 0) groups[i] = 0
    else if (bits < 16) groups[i] &= (0xffff << (16 - bits)) & 0xffff
  }
  return `${groups.map(g => g.toString(16)).join(':')}/${Number(prefix)}`
}

export function isValidIpLiteral(value: string): boolean {
  const s = value.trim()
  if (s.includes(':')) return parseIpv6Groups(s) !== null
  return /^(?:0|[1-9]\d{0,2})(?:\.(?:0|[1-9]\d{0,2})){3}$/.test(s)
    && s.split('.').every(n => Number(n) <= 255)
}
