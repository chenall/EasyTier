<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import { Button, Drawer, ProgressSpinner, useToast, InputSwitch, Popover, Dropdown, Toolbar, MultiSelect, Dialog, InputText } from 'primevue';
import Tooltip from 'primevue/tooltip';
import { useRoute, useRouter } from 'vue-router';
import { Utils } from 'easytier-frontend-lib';
import DeviceDetails from './DeviceDetails.vue';
import { useI18n } from 'vue-i18n'
import ApiClient from '../modules/api';

const { t } = useI18n()

declare const window: Window & typeof globalThis;

// 注册 Tooltip 指令
const vTooltip = Tooltip;

const props = defineProps({
    api: ApiClient,
});

const detailPopover = ref();
const selectedDevice = ref<Utils.DeviceInfo | null>(null);
// 从 localStorage 读取显示详情状态，默认为 false
const showDetailedView = ref(localStorage.getItem('deviceList.showDetailedView') === 'true');

// 监听显示详情状态变化，保存到 localStorage
watch(showDetailedView, (newValue) => {
    localStorage.setItem('deviceList.showDetailedView', newValue.toString());
});

const api = props.api;

const deviceList = ref<Array<Utils.DeviceInfo> | undefined>(undefined);

const selectedDeviceId = computed<string | undefined>(() => route.params.deviceId as string);

const route = useRoute();
const router = useRouter();
const toast = useToast();

const loadDevices = async () => {
    const resp = await api?.list_machines();
    let devices: Array<Utils.DeviceInfo> = [];
    for (const device of (resp || [])) {
        devices.push(Utils.buildDeviceInfo(device));
    }
    console.debug("device list", deviceList.value);
    deviceList.value = devices;
};

const periodFunc = new Utils.PeriodicTask(async () => {
    try {
        await loadDevices();
    } catch (e) {
        toast.add({ severity: 'error', summary: 'Load Device List Failed', detail: e, life: 2000 });
        console.error(e);
    }
}, 1000);

onMounted(async () => {
    periodFunc.start();
    // 初始化屏幕尺寸相关变量
    handleResize();
    window.addEventListener('resize', handleResize);
});

onUnmounted(() => {
    periodFunc.stop();
    window.removeEventListener('resize', handleResize);
});

const deviceManageVisible = computed<boolean>({
    get: () => !!selectedDeviceId.value,
    set: (value) => {
        if (!value) {
            router.push({ name: 'deviceList', params: { deviceId: undefined } });
        }
    }
});

const selectedDeviceHostname = computed<string | undefined>(() => {
    return deviceList.value?.find((device) => device.machine_id === selectedDeviceId.value)?.hostname;
});

// 处理设备管理
const handleDeviceManagement = (device: Utils.DeviceInfo) => {
    const instanceId = device.running_network_instances?.[0];
    router.push({
        name: 'deviceManagement',
        params: {
            deviceId: device.machine_id,
            instanceId: instanceId
        }
    });
};

// 显示设备详情
const showDeviceDetails = (device: Utils.DeviceInfo, event: Event) => {
    selectedDevice.value = device;
    detailPopover.value.toggle(event);
};

// 检查是否为桌面设备
const isDesktop = ref(false);
// 检查是否为多卡片视图（一行可以放置多个卡片）
const isMultiCardView = ref(false);

// 抽屉布局相关
const drawerWidth = computed(() => {
    return isDesktop.value ? 'w-3/5 min-w-96' : 'w-full';
});

const drawerPosition = computed(() => {
    return isDesktop.value ? 'right' : 'bottom';
});

const drawerHeight = computed(() => {
    return isDesktop.value ? undefined : '100%';
});

// 排序相关
const sortOptions = ref([
    { name: () => t('web.device.sort_by_hostname'), value: 'hostname', icon: 'pi pi-home' },
    { name: () => t('web.device.sort_by_version'), value: 'version', icon: 'pi pi-tag' },
    { name: () => t('web.device.sort_by_networks'), value: 'networks', icon: 'pi pi-sitemap' },
    { name: () => t('web.device.sort_by_status'), value: 'status', icon: 'pi pi-circle-fill' }
]);
const selectedSortOption = ref(sortOptions.value[0]);
// 排序方向 (true为升序，false为降序)
const ascending = ref(true);

// 切换排序方向
const toggleSortDirection = () => {
    ascending.value = !ascending.value;
};

// 排序函数
const sortDevices = (devices: Array<Utils.DeviceInfo> | undefined) => {
    if (!devices) return [];

    const sortField = selectedSortOption.value.value;
    const direction = ascending.value ? 1 : -1;

    return [...devices].sort((a, b) => {
        let result = 0;

        switch (sortField) {
            case 'hostname':
                result = a.hostname.localeCompare(b.hostname);
                break;
            case 'version':
                result = a.easytier_version.localeCompare(b.easytier_version);
                break;
            case 'networks':
                result = a.running_network_count - b.running_network_count;
                break;
        case 'status':
                result = (a.online ? 0 : 1) - (b.online ? 0 : 1);
                break;
        }

        return result * direction;
    });
};

// 排序后的设备列表
const sortedDeviceList = computed(() => {
    return sortDevices(deviceList.value);
});

// 标签筛选：根据选中的标签过滤设备
const availableTags = computed<Array<{ name: string; value: string }>>(() => {
    const set = new Set<string>();
    deviceList.value?.forEach((d) => (d.tags ?? []).forEach((t) => set.add(t)));
    return Array.from(set).map((t) => ({ name: t, value: t }));
});
const selectedTags = ref<Array<string>>([]);

const filteredDeviceList = computed<Array<Utils.DeviceInfo> | undefined>(() => {
    const list = sortedDeviceList.value;
    if (!list || selectedTags.value.length === 0) return list;
    return list.filter((d) => (d.tags ?? []).some((t) => selectedTags.value.includes(t)));
});

// 编辑别名与标签
const editDialogVisible = ref(false);
const editingDevice = ref<Utils.DeviceInfo | null>(null);
const editAlias = ref('');
const editTagsText = ref('');

const openEdit = (device: Utils.DeviceInfo) => {
    editingDevice.value = device;
    editAlias.value = device.alias ?? '';
    editTagsText.value = (device.tags ?? []).join(', ');
    editDialogVisible.value = true;
};

const saveEdit = async () => {
    if (!editingDevice.value) return;
    const mid = editingDevice.value.machine_id;
    const tags = editTagsText.value
        .split(',')
        .map((s) => s.trim())
        .filter((s) => s.length > 0);
    try {
        await api?.set_machine_alias(mid, editAlias.value);
        await api?.set_machine_tags(mid, tags);
        editDialogVisible.value = false;
        await loadDevices();
        toast.add({ severity: 'success', summary: t('web.device.save_success'), life: 2000 });
    } catch (e) {
        toast.add({ severity: 'error', summary: t('web.device.save_failed'), detail: e, life: 2000 });
    }
};

// 保存resize事件处理函数的引用，以便正确移除
const handleResize = () => {
    isDesktop.value = window.innerWidth >= 768;
    // 当容器宽度足够放置两个或更多卡片时，视为多卡片视图
    isMultiCardView.value = window.innerWidth >= 650;
};

</script>

<style scoped>
/* 卡片容器 */
.card-container {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
    gap: 1rem;
    width: 100%;
    position: relative;
    /* 确保子元素的绝对定位相对于此容器 */
}

/* 设备卡片样式 */
.device-card {
    border: 1px solid var(--surface-border, #e5e7eb);
    border-radius: 0.5rem;
    background: var(--surface-card, white);
    box-shadow: 0 1px 3px 0 rgba(0, 0, 0, 0.1);
    transition: transform 0.2s ease, box-shadow 0.2s ease, background-color 0.3s ease;
    display: flex;
    flex-direction: column;
    position: relative;
    overflow: hidden;
}

.device-card:hover {
    transform: translateY(-2px);
    box-shadow: 0 4px 6px -1px rgba(0, 0, 0, 0.1), 0 2px 4px -1px rgba(0, 0, 0, 0.06);
}

.card-header {
    padding: 0.75rem;
    display: flex;
    flex-direction: column;
    position: relative;
    color: var(--text-color, #1f2937);
}

.device-details-popover {
    min-width: 280px;
    max-width: 350px;
    padding: 0.3rem;
}

/* Popover 样式 */
:deep(.device-popover.p-popover) {
    min-width: 320px;
    border-radius: 0.5rem;
    box-shadow: var(--card-shadow, 0 10px 15px -3px rgba(0, 0, 0, 0.1), 0 4px 6px -2px rgba(0, 0, 0, 0.05));
    border: 1px solid var(--surface-border, #e5e7eb);
    overflow: hidden;
}

:deep(.device-popover .p-popover-content) {
    padding: 0;
    background-color: var(--surface-card, #ffffff);
    color: var(--text-color, #334155);
}

:deep(.device-popover .p-popover-arrow) {
    background-color: var(--surface-card, #ffffff);
    border-color: var(--surface-border, #e5e7eb);
}

:deep(.device-popover .p-popover-header) {
    background-color: var(--surface-section, #f8fafc);
    border-bottom: 1px solid var(--surface-border, #e2e8f0);
}

:deep(.device-popover .p-popover-header-close) {
    color: var(--text-color-secondary, #64748b);
}

:deep(.device-popover .p-popover-header-close:hover) {
    background-color: var(--surface-hover, rgba(0, 0, 0, 0.04));
    color: var(--text-color, #334155);
    border-radius: 50%;
}

@media (prefers-color-scheme: dark) {
    :deep(.device-popover.p-popover) {
        box-shadow: 0 10px 15px -3px rgba(0, 0, 0, 0.5), 0 4px 6px -2px rgba(0, 0, 0, 0.25);
        border-color: var(--surface-border, #334155);
    }

    :deep(.device-popover .p-popover-content) {
        background-color: var(--surface-card, #1e293b);
        color: var(--text-color, #f1f5f9);
    }

    :deep(.device-popover .p-popover-arrow) {
        background-color: var(--surface-card, #1e293b);
        border-color: var(--surface-border, #334155);
    }

    :deep(.device-popover .p-popover-header) {
        background-color: var(--surface-section, #0f172a);
        border-bottom: 1px solid var(--surface-border, #1e293b);
    }

    :deep(.device-popover .p-popover-header-close) {
        color: var(--text-color-secondary, #94a3b8);
    }

    :deep(.device-popover .p-popover-header-close:hover) {
        background-color: var(--surface-hover, rgba(255, 255, 255, 0.1));
        color: var(--text-color, #f1f5f9);
    }

    .popover-header {
        background-color: var(--surface-section, #0f172a);
        color: var(--text-color, #f1f5f9);
        border-bottom: 1px solid var(--surface-border, #334155);
    }
}

.popover-header {
    display: flex;
    align-items: center;
    background-color: var(--surface-section, #f8fafc);
    padding: 0.75rem 1rem;
    border-bottom: 1px solid var(--surface-border, #e2e8f0);
    color: var(--text-color, #334155);
}

/* 卡片内详情样式 */
.card-details {
    background-color: var(--surface-ground, #f9fafb);
}

/* 卡片内详情内容的特定样式 */
:deep(.card-details-content) {
    padding: 0.15rem 0.1rem;
}

/* 卡片中的紧凑详情内容 */
:deep(.card-details-content) {
    padding: 0.15rem 0.1rem;
}

:deep(.card-details-content .detail-label) {
    font-size: 0.9rem;
}

:deep(.card-details-content .detail-value) {
    font-size: 0.85rem;
}

@media (prefers-color-scheme: dark) {
    :deep(.card-details-content .detail-item) {
        border-bottom: 1px solid var(--surface-border, #334155);
    }

    :deep(.card-details-content .detail-item:last-child) {
        border-bottom: none;
    }

    :deep(.card-details-content .detail-item:hover) {
        background-color: var(--surface-hover, rgba(30, 41, 59, 0.4));
    }

    :deep(.card-details-content .detail-label) {
        color: var(--text-color, #e2e8f0);
    }

    :deep(.card-details-content .detail-value) {
        color: var(--text-color-secondary, #cbd5e1);
    }
}

@media (prefers-color-scheme: dark) {
    :deep(.card-details-content .detail-item) {
        border-bottom: 1px solid var(--surface-border, #334155);
    }

    :deep(.card-details-content .detail-item:last-child) {
        border-bottom: none;
    }

    :deep(.card-details-content .detail-item:hover) {
        background-color: var(--surface-hover, rgba(30, 41, 59, 0.4));
    }

    :deep(.card-details-content .detail-label) {
        color: var(--text-color, #e2e8f0);
    }

    :deep(.card-details-content .detail-value) {
        color: var(--text-color-secondary, #cbd5e1);
    }
}

/* 确保卡片在暗黑模式下有足够的对比度 */
:deep(.device-card) {
    background-color: var(--surface-card, white);
    border-color: var(--surface-border, #e5e7eb);
}

:deep(.card-header) {
    color: var(--text-color, #1f2937);
}

.card-title {
    color: var(--text-color, #1f2937);
}

.card-subtitle {
    color: var(--text-color-secondary, #64748b);
}

.version-badge {
    background-color: var(--primary-color, #3b82f6);
    color: #ffffff;
    padding: 0.1rem 0.4rem;
    border-radius: 0.75rem;
    font-weight: 500;
    letter-spacing: 0.02em;
    font-size: 0.65rem;
}

.sort-controls {
    background-color: var(--surface-card);
    border-radius: 0.5rem;
    padding: 0.25rem 0.5rem;
    box-shadow: var(--card-shadow, 0 1px 3px rgba(0, 0, 0, 0.05));
    transition: all 0.2s;
}

.sort-controls:hover {
    box-shadow: var(--card-shadow, 0 2px 5px rgba(0, 0, 0, 0.1));
}

.sort-label {
    font-weight: 500;
    color: var(--text-color-secondary);
}

.sort-dropdown {
    min-width: 6rem;
    max-width: 9rem;
}

.sort-icon {
    font-size: 0.8rem;
}

.sort-direction-btn {
    font-size: 1rem;
    width: 2.5rem !important;
    height: 2.5rem !important;
}

/* 暗黑模式样式适配 */
@media (prefers-color-scheme: dark) {
    .sort-controls {
        background-color: var(--surface-card, #1e293b);
        box-shadow: 0 1px 3px rgba(0, 0, 0, 0.2);
    }

    .sort-controls:hover {
        box-shadow: 0 2px 5px rgba(0, 0, 0, 0.25);
    }

    :deep(.device-card) {
        background-color: var(--surface-card, #1e293b);
        border-color: var(--surface-border, #334155);
        box-shadow: 0 1px 3px 0 rgba(0, 0, 0, 0.3);
    }

    :deep(.card-header) {
        color: var(--text-color, #f1f5f9);
    }

    .card-title {
        color: var(--text-color, #f1f5f9);
    }

    .card-subtitle {
        color: var(--text-color-secondary, #cbd5e1);
    }

    .version-badge {
        background-color: var(--primary-color, #4f46e5);
    }

    :deep(.card-details) {
        background-color: var(--surface-ground, #0f172a);
        border-top: 1px solid var(--surface-border, #334155);
    }
}

/* Popover 详情内容的特定样式 */
:deep(.popover-details-content) {
    padding: 0.25rem 0.2rem;
    max-width: 320px;
}

/* Popover 中的紧凑详情内容 */
:deep(.popover-details-content) {
    padding: 0.25rem 0.2rem;
    max-width: 320px;
}

:deep(.popover-details-content .detail-label) {
    font-size: 0.8rem;
}

:deep(.popover-details-content .detail-value) {
    font-size: 0.8rem;
}

:deep(.popover-details-content .machine-id-value) {
    font-size: 0.7rem;
}

@media (prefers-color-scheme: dark) {
    :deep(.popover-details-content .detail-item) {
        border-bottom: 1px solid var(--surface-border, #334155);
    }

    :deep(.popover-details-content .detail-item:last-child) {
        border-bottom: none;
    }

    :deep(.popover-details-content .detail-item:hover) {
        background-color: var(--surface-hover, rgba(30, 41, 59, 0.4));
    }

    :deep(.popover-details-content .detail-label) {
        color: var(--text-color, #e2e8f0);
    }

    :deep(.popover-details-content .detail-value) {
        color: var(--text-color-secondary, #cbd5e1);
    }
}

@media (prefers-color-scheme: dark) {
    :deep(.popover-details-content .detail-item) {
        border-bottom: 1px solid var(--surface-border, #334155);
    }

    :deep(.popover-details-content .detail-item:last-child) {
        border-bottom: none;
    }

    :deep(.popover-details-content .detail-item:hover) {
        background-color: var(--surface-hover, rgba(30, 41, 59, 0.4));
    }

    :deep(.popover-details-content .detail-label) {
        color: var(--text-color, #e2e8f0);
    }

    :deep(.popover-details-content .detail-value) {
        color: var(--text-color-secondary, #cbd5e1);
    }
}

/* 移动端卡片样式 */
@media (max-width: 768px) {
    .card-container {
        grid-template-columns: 1fr;
    }
}

/* 动画效果 */
@keyframes fadeIn {
    from {
        opacity: 0;
        transform: translateY(-10px);
    }

    to {
        opacity: 1;
        transform: translateY(0);
    }
}

.fade-in {
    animation: fadeIn 0.3s ease-out;
}

/* 抽屉响应式样式 */
:deep(.p-drawer) {
    transition: all 0.3s ease;
}

:deep(.p-drawer.p-drawer-bottom) {
    border-top-left-radius: 1rem;
    border-top-right-radius: 1rem;
    box-shadow: 0 -4px 6px -1px rgba(0, 0, 0, 0.1);
}

:deep(.p-drawer.p-drawer-bottom .p-drawer-header) {
    padding-top: 1rem;
    border-top-left-radius: 1rem;
    border-top-right-radius: 1rem;
}

:deep(.p-drawer.p-drawer-bottom .p-drawer-content) {
    padding-bottom: 2rem;
    border-top-left-radius: 1rem;
    border-top-right-radius: 1rem;
}

/* 底部抽屉的拖动指示器 */
:deep(.p-drawer.p-drawer-bottom .p-drawer-header::before) {
    content: "";
    position: absolute;
    top: 0.5rem;
    left: 50%;
    transform: translateX(-50%);
    width: 4rem;
    height: 4px;
    background-color: var(--surface-border);
    border-radius: 2px;
    opacity: 0.8;
}

@media (prefers-color-scheme: dark) {
    :deep(.p-drawer.p-drawer-bottom) {
        box-shadow: 0 -4px 12px -1px rgba(0, 0, 0, 0.3);
    }
}

.drawer-fab-close-btn {
    /* 适配移动和桌面端，防止被内容遮挡 */
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.18);
    transition: box-shadow 0.2s;
}

.drawer-fab-close-btn:hover {
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.22);
}

/* 排序控件在小屏幕下单独一行 */
.sort-controls-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
}

@media (max-width: 640px) {
    .sort-controls-row {
        flex-direction: column;
        align-items: stretch;
        gap: 0.5rem;
        width: 100%;
        margin-top: 0.5rem;
    }

    .sort-controls {
        width: 100%;
        justify-content: flex-start;
    }
}

/* 工具栏样式优化 */
:deep(.p-dropdown) {
    background: transparent;
    border: 1px solid var(--surface-border);
    transition: all 0.2s;
}

:deep(.p-dropdown:hover) {
    border-color: var(--primary-color);
}

:deep(.p-dropdown-panel) {
    .p-dropdown-items .p-dropdown-item {
        padding: 0.75rem 1rem;
    }
}

:deep(.p-inputswitch) {
    .p-inputswitch-slider {
        background: var(--surface-200);
    }
}

/* 确保所有按钮大小一致 */
:deep(.p-button.p-button-icon-only) {
    width: 2.5rem;
    height: 2.5rem;
}

/* 位置样式 */
.location-icon {
    color: var(--pink-500);
    font-size: 0.9rem;
}

.location-text {
    font-size: 0.875rem;
    line-height: 1.25rem;
    opacity: 0.9;
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
}

.location-separator {
    opacity: 0.5;
    font-weight: 300;
    margin: 0 0.1rem;
}

/* 在线/离线状态点 */
.status-dot {
    display: inline-block;
    width: 0.55rem;
    height: 0.55rem;
    border-radius: 50%;
    flex-shrink: 0;
}

.status-dot.online {
    background-color: #22c55e;
    box-shadow: 0 0 0 3px rgba(34, 197, 94, 0.2);
}

.status-dot.offline {
    background-color: #94a3b8;
}

/* 标签 chips */
.device-tag-chip {
    font-size: 0.7rem;
    line-height: 1.2;
    padding: 0.1rem 0.5rem;
    border-radius: 0.75rem;
    background-color: var(--primary-color-50, #eff6ff);
    color: var(--primary-color-700, #1d4ed8);
    border: 1px solid var(--primary-color-200, #bfdbfe);
    white-space: nowrap;
}

@media (prefers-color-scheme: dark) {
    .device-tag-chip {
        background-color: rgba(59, 130, 246, 0.15);
        color: #93c5fd;
        border-color: rgba(59, 130, 246, 0.4);
    }

    .status-dot.offline {
        background-color: #64748b;
    }
}

@media (prefers-color-scheme: dark) {
    .location-text {
        color: var(--text-color-secondary, #cbd5e1);
    }

    .location-icon {
        color: var(--pink-400);
    }
}
</style>

<template>
    <div class="flex flex-col gap-4">
        <!-- 标题和工具栏 -->
        <div class="text-xl font-bold">
            <h1>{{ t('web.device.list') }}</h1>
        </div>

        <Toolbar class="mb-4 p-3 gap-4 surface-0 border-1 surface-border rounded-md">
            <template #start>
                <div class="flex items-center gap-2">
                    <label for="sort-by" class="text-sm text-500 hidden sm:block">{{ t('web.device.sort_by') }}：</label>
                    <Dropdown id="sort-by" v-model="selectedSortOption" :options="sortOptions" optionLabel="name"
                        class="sort-dropdown text-sm !min-w-[120px] sm:!min-w-[140px]" panelClass="text-sm">
                        <template #value="slotProps">
                            <div class="flex items-center gap-2">
                                <i :class="[slotProps.value.icon, 'text-600']"></i>
                                <span class="text-600">{{ slotProps.value.name() }}</span>
                            </div>
                        </template>
                        <template #option="slotProps">
                            <div class="flex items-center gap-2">
                                <i :class="[slotProps.option.icon, 'text-600']"></i>
                                <span>{{ slotProps.option.name() }}</span>
                            </div>
                        </template>
                    </Dropdown>
                    <Button :icon="ascending ? 'pi pi-sort-amount-up' : 'pi pi-sort-amount-down'" severity="secondary"
                        text rounded class="sort-direction-btn min-w-[2.5rem] h-[2.5rem]"
                        v-tooltip.top="ascending ? t('web.device.sort_direction_asc') : t('web.device.sort_direction_desc')"
                        @click="toggleSortDirection" />
                </div>
                <div class="flex items-center gap-2 ml-2">
                    <label class="text-sm text-500 hidden md:block">{{ t('web.device.filter_by_tag') }}：</label>
                    <MultiSelect v-model="selectedTags" :options="availableTags" optionLabel="name"
                        optionValue="value" :placeholder="t('web.device.all_tags')" display="chip"
                        :maxSelectedLabels="3" class="!min-w-[140px] text-sm" />
                </div>
            </template>
            <template #end>
                <div class="flex items-center gap-3">
                    <div class="hidden sm:block border-r-1 surface-border h-4 mr-2"></div>
                    <div class="flex items-center gap-2">
                        <label for="detailed-view" class="text-sm text-500 hidden sm:block">{{
                            t('web.device.show_detailed_view') }}</label>
                        <InputSwitch id="detailed-view" v-model="showDetailedView" />
                    </div>
                </div>
            </template>
        </Toolbar>

        <div v-if="deviceList === undefined" class="w-full flex justify-center">
            <ProgressSpinner />
        </div>

        <div v-if="deviceList !== undefined">
            <!-- 卡片视图 (适用于所有屏幕尺寸) -->
            <div class="card-container">
                <div v-for="device in filteredDeviceList" :key="device.machine_id" class="device-card">
                    <!-- 卡片头部 -->
                    <div class="card-header">
                        <!-- 上部区域：设备名称和版本徽章 -->
                        <div class="flex justify-between items-center mb-2">
                            <!-- 设备名称（别名优先）与在线状态 -->
                            <div class="flex items-center gap-2 min-w-0">
                                <span class="status-dot" :class="device.online ? 'online' : 'offline'"
                                    :title="device.online ? t('web.device.online') : t('web.device.offline')"></span>
                                <div class="font-semibold truncate card-title"
                                    :title="device.alias || device.hostname">{{ device.alias || device.hostname }}
                                </div>
                            </div>

                            <!-- 版本徽章 -->
                            <div class="text-xs version-badge" v-tooltip="`EasyTier ${device.easytier_version}`">
                                v{{ (device.easytier_version || '').split('-')[0] }}
                            </div>
                        </div>

                        <!-- 下部区域：IP地址和操作按钮 -->
                        <div class="flex justify-between items-center">
                            <!-- IP地址和位置信息 -->
                            <div class="text-sm truncate card-subtitle max-w-[60%] flex items-center gap-2"
                                :title="device.location ? `${device.location.country}${device.location.region ? ' · ' + device.location.region : ''}${device.location.city ? ' · ' + device.location.city : ''}` : t('web.device.unknown_location')">
                                <i class="pi pi-map-marker location-icon"></i>
                                <span class="location-text">
                                    <template v-if="device.location">
                                        {{ device.location.country }}
                                        <template v-if="device.location.region">
                                            <span class="location-separator">·</span>
                                            {{ device.location.region }}
                                        </template>
                                        <template v-if="device.location.city">
                                            <span class="location-separator">·</span>
                                            {{ device.location.city }}
                                        </template>
                                    </template>
                                    <template v-else>
                                        {{ t('web.device.unknown_location') }}
                                    </template>
                                </span>
                            </div>

                            <!-- 操作按钮组 -->
                            <div class="flex items-center space-x-2">
                                <!-- 网络数量徽章 -->
                                <span v-tooltip="t('web.device.network_count')"
                                    class="inline-flex items-center justify-center w-6 h-6 text-xs font-medium bg-blue-100 text-blue-800 rounded-full">
                                    {{ device.running_network_count }}
                                </span>

                                <!-- 详情按钮 -->
                                <Button v-tooltip="t('web.device.show_detailed_view')" icon="pi pi-info-circle"
                                    severity="info" text rounded class="w-9 h-9" v-if="!showDetailedView"
                                    @click="showDeviceDetails(device, $event)" />

                                <!-- 编辑别名/标签 -->
                                <Button icon="pi pi-pencil" @click="openEdit(device)" severity="secondary" text
                                    rounded class="w-9 h-9" :title="t('web.device.edit')" />

                                <!-- 设置按钮 -->
                                <Button icon="pi pi-cog" @click="handleDeviceManagement(device)" severity="secondary"
                                    rounded class="w-9 h-9" :title="`Manage ${device.alias || device.hostname}`" />
                            </div>
                        </div>
                    </div>

                    <!-- 标签 chips -->
                    <div v-if="(device.tags ?? []).length" class="flex flex-wrap gap-1 px-3 pb-2">
                        <span v-for="tag in device.tags" :key="tag" class="device-tag-chip">{{ tag }}</span>
                    </div>

                    <!-- 详情区域 - 当开启详情显示时展示 -->
                    <div v-if="showDetailedView" class="card-details border-t border-gray-200 fade-in">
                        <DeviceDetails :device="device" containerClass="card-details-content" :compact="true" />
                    </div>
                </div>
            </div>
        </div>

        <!-- 全局设备详情 Popover -->
        <Popover ref="detailPopover" :showCloseIcon="true" :closeOnEscape="true" :autoHide="false" appendTo="body"
            class="device-popover">
            <template v-if="selectedDevice">
                <div class="popover-header">
                    <i class="pi pi-info-circle mr-2"></i>
                    <span class="font-bold">设备详情</span>
                </div>
                <div class="device-details-popover">
                    <DeviceDetails :device="selectedDevice" containerClass="popover-details-content" :compact="true" />
                </div>
            </template>
        </Popover>

        <Drawer v-model:visible="deviceManageVisible" :position="drawerPosition"
            :header="`Manage ${selectedDeviceHostname}`" :baseZIndex=1000 class="" :class="drawerWidth"
            :style="{ height: drawerHeight }">
            <template #container="{ closeCallback }">
                <div style="position: relative; height: 100%;" class="device-manage-drawer">
                    <RouterView v-slot="{ Component }">
                        <component :is="Component" :api="api" :deviceList="deviceList" @update="loadDevices" />
                    </RouterView>
                    <Button icon="pi pi-times" rounded severity="danger"
                        class="fixed z-50 right-6 bottom-6 shadow-lg drawer-fab-close-btn"
                        style="width: 3.2rem; height: 3.2rem; font-size: 1.5rem;" @click="closeCallback" />
                </div>
            </template>
        </Drawer>

        <!-- 编辑别名与标签 -->
        <Dialog v-model:visible="editDialogVisible" :header="t('web.device.edit_alias_tags')" modal
            :draggable="false" class="w-[90%] max-w-[420px]">
            <div class="flex flex-col gap-4">
                <div class="flex flex-col gap-1">
                    <label class="text-sm font-medium">{{ t('web.device.alias') }}</label>
                    <InputText v-model="editAlias" :placeholder="t('web.device.alias_placeholder')" class="w-full" />
                </div>
                <div class="flex flex-col gap-1">
                    <label class="text-sm font-medium">{{ t('web.device.tags') }}</label>
                    <InputText v-model="editTagsText" :placeholder="t('web.device.tags_placeholder')" class="w-full" />
                    <small class="text-500">{{ t('web.device.tags_hint') }}</small>
                </div>
            </div>
            <template #footer>
                <div class="flex justify-end gap-2">
                    <Button :label="t('web.common.cancel')" severity="secondary" text
                        @click="editDialogVisible = false" />
                    <Button :label="t('web.common.save')" @click="saveEdit" />
                </div>
            </template>
        </Dialog>
    </div>
</template>
