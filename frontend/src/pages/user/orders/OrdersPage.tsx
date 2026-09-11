import { Link } from "react-router-dom"
import { CreditCard, ReceiptText, RefreshCw } from "lucide-react"
import {
  useOrders,
  OrdersTable,
  OrderDetailDrawer,
} from "@/features/user-orders"
import { userRoutes } from "@/shared/routes/user"

export default function Orders() {
  const {
    orders,
    orderLoading,
    orderPage,
    setOrderPage,
    orderTotal,
    orderFilterStatus,
    handleOrderFilter,
    selectedOrder,
    setSelectedOrder,
    loadPaymentOrders,
  } = useOrders()

  const orderTotalPages = Math.ceil(orderTotal / 20)

  return (
    <div className="space-y-6">
      {/* Page Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl sm:text-3xl font-extrabold tracking-tight text-foreground">
            充值订单
          </h1>
          <p className="text-sm text-muted-foreground mt-1">
            账户在线充值流水、多渠道支付凭据与实时入账明细
          </p>
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => loadPaymentOrders(orderPage)}
            disabled={orderLoading}
            aria-label="刷新订单列表"
            className="btn btn-secondary h-9 px-3.5 text-xs font-semibold rounded-xl border-border shadow-2xs gap-1.5"
          >
            <RefreshCw className={`w-3.5 h-3.5 ${orderLoading ? "animate-spin text-primary" : ""}`} />
            <span>刷新</span>
          </button>
          <Link
            to={userRoutes.finance}
            className="btn btn-secondary h-9 px-3.5 text-xs font-semibold rounded-xl border-border shadow-2xs gap-1.5"
          >
            <ReceiptText className="w-3.5 h-3.5 text-muted-foreground" />
            <span>财务中心</span>
          </Link>
          <Link
            to={userRoutes.financeTopup}
            className="btn btn-primary h-9 px-4 text-xs font-semibold rounded-xl shadow-xs gap-1.5"
          >
            <CreditCard className="w-3.5 h-3.5" />
            <span>在线充值</span>
          </Link>
        </div>
      </div>

      {/* Orders Table and Filtering */}
      <OrdersTable
        orders={orders}
        loading={orderLoading}
        page={orderPage}
        totalPages={orderTotalPages}
        total={orderTotal}
        filterStatus={orderFilterStatus}
        onFilter={handleOrderFilter}
        onPageChange={setOrderPage}
        onRefresh={loadPaymentOrders}
        onSelectOrder={setSelectedOrder}
      />

      {/* Digital Receipt Drawer */}
      {selectedOrder && (
        <OrderDetailDrawer order={selectedOrder} onClose={() => setSelectedOrder(null)} />
      )}
    </div>
  )
}

