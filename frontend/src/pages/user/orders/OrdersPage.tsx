import { Link } from "react-router-dom"
import { Package } from "lucide-react"
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
    <div className="space-y-6 animate-in fade-in slide-in-from-bottom-4 duration-500" style={{ willChange: 'transform, opacity' }}>
      <div>
        <h2 className="text-2xl font-bold tracking-tight text-gray-900 dark:text-white flex items-center gap-2">
          <Package className="w-6 h-6 text-primary" />
          充值订单
        </h2>
        <p className="text-gray-500 dark:text-dark-300 mt-1">
          查看在线支付充值记录。订阅权益与退订请前往{' '}
          <Link to={userRoutes.subscriptions} className="text-primary-600 hover:underline">
            订阅
          </Link>
          。
        </p>
      </div>

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

      {selectedOrder && (
        <OrderDetailDrawer order={selectedOrder} onClose={() => setSelectedOrder(null)} />
      )}
    </div>
  )
}
