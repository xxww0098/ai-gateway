// Types for tickets feature module

/** Ticket list item (user and admin views) */
export interface TicketItem {
  id: number
  title: string
  category: string
  status: string
  priority: string
  user_id?: number
  user_email?: string
  assigned_to?: number | null
  created_at: string
  updated_at: string
}

/** Single reply within a ticket conversation */
interface TicketReply {
  id: number
  user_id: number
  content: string
  is_staff: boolean
  created_at: string
}

/** Full ticket detail including replies */
export interface TicketDetail {
  id: number
  title: string
  category: string
  status: string
  priority: string
  user_id: number
  user_email?: string
  assigned_to?: number | null
  created_at: string
  updated_at: string
  closed_at?: string | null
  replies: TicketReply[]
}

/** Request payload for creating a new ticket */
export interface CreateTicketRequest {
  title: string
  category: string
  priority: string
  content: string
}

/** Request payload for replying to a ticket */
export interface ReplyTicketRequest {
  content: string
}

/** Request payload for updating ticket status (admin) */
export interface UpdateTicketStatusRequest {
  status: string
}

/** Request payload for assigning a ticket (admin) */
export interface AssignTicketRequest {
  staff_id: number
}

/** Quick reply item used in admin ticket panel */
export interface TicketQuickReplyItem {
  label: string
  text: string
}
