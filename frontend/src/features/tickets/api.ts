// API functions for tickets feature module
import { apiClient } from "@/shared/api/client"
import type { PaginatedResponse } from "@/shared/types/api"
import type {
  TicketItem,
  TicketDetail,
  CreateTicketRequest,
  ReplyTicketRequest,
  UpdateTicketStatusRequest,
  AssignTicketRequest,
  TicketQuickReplyItem,
} from "./types"

// ── User Ticket Endpoints ───────────────────────────────────────────────────

export function fetchUserTickets(page: number, pageSize: number, status?: string) {
  const params = new URLSearchParams()
  params.set("page", String(page))
  params.set("page_size", String(pageSize))
  if (status) params.set("status", status)
  return apiClient.get<PaginatedResponse<TicketItem>>(`/user/tickets?${params}`)
}

export function fetchUserTicketDetail(id: number | string) {
  return apiClient.get<TicketDetail>(`/user/tickets/${id}`)
}

export function createTicket(body: CreateTicketRequest) {
  return apiClient.post<TicketItem>(`/user/tickets`, body)
}

export function replyUserTicket(ticketId: number | string, body: ReplyTicketRequest) {
  return apiClient.post(`/user/tickets/${ticketId}/replies`, body)
}

// ── Admin Ticket Endpoints ──────────────────────────────────────────────────

export function fetchAdminTickets(page: number, pageSize: number, status?: string) {
  const params = new URLSearchParams()
  params.set("page", String(page))
  params.set("page_size", String(pageSize))
  if (status) params.set("status", status)
  return apiClient.get<PaginatedResponse<TicketItem>>(`/admin/tickets?${params}`)
}

export function fetchAdminTicketDetail(id: number | string) {
  return apiClient.get<TicketDetail>(`/admin/tickets/${id}`)
}

export function replyAdminTicket(ticketId: number | string, body: ReplyTicketRequest) {
  return apiClient.post(`/admin/tickets/${ticketId}/replies`, body)
}

export function updateTicketStatus(ticketId: number | string, body: UpdateTicketStatusRequest) {
  return apiClient.put(`/admin/tickets/${ticketId}/status`, body)
}

export function assignTicket(ticketId: number | string, body: AssignTicketRequest) {
  return apiClient.put(`/admin/tickets/${ticketId}/assign`, body)
}

// ── Admin Quick Replies ─────────────────────────────────────────────────────

export function fetchTicketQuickReplies() {
  return apiClient.get<{ items: TicketQuickReplyItem[] }>(`/admin/ticket-quick-replies`)
}

export function saveTicketQuickReplies(items: TicketQuickReplyItem[]) {
  return apiClient.post<{ items: TicketQuickReplyItem[] }>(`/admin/ticket-quick-replies`, { items })
}
