import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query"
import { queryKeys } from "@/shared/api/query-keys"
import { errorMessage } from "@/shared/api/errors"
import { toast } from "sonner"
import type { CreateTicketRequest, ReplyTicketRequest } from "./types"
import {
  fetchUserTickets,
  fetchUserTicketDetail,
  createTicket,
  replyUserTicket,
  fetchAdminTickets,
  fetchAdminTicketDetail,
  replyAdminTicket,
  updateTicketStatus,
  assignTicket,
} from "./api"

// ── User Hooks ──────────────────────────────────────────────────────────────

/** Fetches paginated user tickets with optional status filter */
export function useTickets(page: number, pageSize = 20, status?: string) {
  return useQuery({
    queryKey: [...queryKeys.tickets.list({ page, pageSize }), status],
    queryFn: () => fetchUserTickets(page, pageSize, status),
  })
}

/** Fetches a single user ticket detail by ID */
export function useTicketDetail(ticketId: string | number | undefined) {
  return useQuery({
    queryKey: ["tickets", "detail", ticketId],
    queryFn: () => fetchUserTicketDetail(ticketId!),
    enabled: !!ticketId,
  })
}

/** Creates a new ticket */
export function useCreateTicket() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (body: CreateTicketRequest) => createTicket(body),
    onSuccess: () => {
      toast.success("工单已创建")
      qc.invalidateQueries({ queryKey: queryKeys.tickets.all() })
    },
    onError: (err) => toast.error(errorMessage(err, "创建失败")),
  })
}

/** Replies to a user ticket */
export function useReplyTicket() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ ticketId, body }: { ticketId: string | number; body: ReplyTicketRequest }) =>
      replyUserTicket(ticketId, body),
    onSuccess: () => {
      toast.success("已发送")
      qc.invalidateQueries({ queryKey: queryKeys.tickets.all() })
    },
    onError: (err) => toast.error(errorMessage(err, "发送失败")),
  })
}

// ── Admin Hooks ─────────────────────────────────────────────────────────────

/** Fetches paginated admin tickets with optional status filter */
export function useAdminTickets(page: number, pageSize = 20, status?: string) {
  return useQuery({
    queryKey: [...queryKeys.tickets.list({ page, pageSize }), "admin", status],
    queryFn: () => fetchAdminTickets(page, pageSize, status),
  })
}

/** Fetches a single admin ticket detail by ID */
export function useAdminTicketDetail(ticketId: string | number | undefined) {
  return useQuery({
    queryKey: ["tickets", "admin", "detail", ticketId],
    queryFn: () => fetchAdminTicketDetail(ticketId!),
    enabled: !!ticketId,
  })
}

/** Replies to a ticket as admin */
export function useAdminReplyTicket() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ ticketId, body }: { ticketId: string | number; body: ReplyTicketRequest }) =>
      replyAdminTicket(ticketId, body),
    onSuccess: () => {
      toast.success("回复成功")
      qc.invalidateQueries({ queryKey: queryKeys.tickets.all() })
    },
    onError: (err) => toast.error(errorMessage(err, "回复失败")),
  })
}

/** Updates ticket status (admin) */
export function useUpdateTicketStatus() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ ticketId, status }: { ticketId: string | number; status: string }) =>
      updateTicketStatus(ticketId, { status }),
    onSuccess: () => {
      toast.success("状态已更新")
      qc.invalidateQueries({ queryKey: queryKeys.tickets.all() })
    },
    onError: (err) => toast.error(errorMessage(err, "更新状态失败")),
  })
}

/** Assigns a ticket to staff (admin) */
export function useAssignTicket() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ ticketId, staffId }: { ticketId: string | number; staffId: number }) =>
      assignTicket(ticketId, { staff_id: staffId }),
    onSuccess: () => {
      toast.success("工单已分配")
      qc.invalidateQueries({ queryKey: queryKeys.tickets.all() })
    },
    onError: (err) => toast.error(errorMessage(err, "分配失败")),
  })
}
