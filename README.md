# mcping-mqtt

A simple Minecraft server ping script that publishes the results to an MQTT broker.

wip

program expects the following environment variables to be set:
```
MQTT_HOST
MQTT_USERNAME
MQTT_PASSWORD
DATABASE_URL
```

# API
this program uses a mqtt based api to create and delete servers. the api is as follows:

## creating servers

send a message to the topic `mcping/create` with the payload `{"name": "server name", "address": "server address"}` to create a server.

listen to mcping/create for details about the success or failure of adding a server. 

## deleting servers

send the message 'delete' to the topic `mcping/<server name>` to delete a server. 

listen to the same topic for details about the success or failure of deleting a server.. a deleted server will get the message `deleted`, a failed deletion will get the message `failed to delete`.