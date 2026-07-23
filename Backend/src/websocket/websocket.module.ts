import { Module } from '@nestjs/common';
import { WebsocketGateway } from './websocket.gateway';
import { PresenceService } from './presence.service';
import { ObservabilityModule } from '../observability/observability.module';
import { RedisModule } from '../redis/redis.module';
import { RedisIoAdapter } from './redis-io.adapter';

@Module({
  imports: [ObservabilityModule, RedisModule],
  providers: [WebsocketGateway, PresenceService, RedisIoAdapter],
  exports: [WebsocketGateway, PresenceService, RedisIoAdapter],
})
export class WebsocketModule {}
