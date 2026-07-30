<script setup lang="ts">
import { useRouter } from 'vue-router';
import ApiClient from '../modules/api';
import { PresetNetworkDialog, type NetworkTypes } from 'easytier-frontend-lib';

defineProps<{ api: ApiClient }>();

const router = useRouter();

// 点击某个分组 -> 进入该分组的设备视图（复用 DeviceList 的分组模式）
function viewGroup(preset: NetworkTypes.PresetSummary) {
    router.push({ name: 'networkGroupDevices', params: { groupId: String(preset.id) } });
}
</script>

<template>
    <div class="flex flex-col gap-4">
        <PresetNetworkDialog :api="api" @view-group="viewGroup" />
    </div>
</template>
