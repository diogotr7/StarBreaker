using System.Diagnostics;
using System.Text;
using System.Text.Json;
using StarBreaker.Common;

namespace StarBreaker.DataCore;

public sealed class DataCoreBinaryJson : IDataCoreBinary<string>
{
    public DataCoreDatabase Database { get; }

    public DataCoreBinaryJson(DataCoreDatabase db)
    {
        Database = db;
    }

    public void SaveRecordToFile(DataCoreRecord record, string path)
    {
        using var fileStream = new FileStream(Path.ChangeExtension(path, "json"), FileMode.Create);
        using var writer = new Utf8JsonWriter(fileStream, new JsonWriterOptions
        {
            Indented = true,
        });

        WriteInner(record, writer);
    }

    public void SaveStructToFile(int structIndex, string path)
    {
        using var fileStream = new FileStream(Path.ChangeExtension(path, "json"), FileMode.Create);
        using var writer = new Utf8JsonWriter(fileStream, new JsonWriterOptions
        {
            Indented = true,
        });
        
        var structDefinition = Database.StructDefinitions[structIndex];
        
        writer.WriteStartObject();
        writer.WriteString("Name", structDefinition.GetName(Database));
        if (structDefinition.ParentTypeIndex != -1)
            writer.WriteString("Parent", Database.StructDefinitions[structDefinition.ParentTypeIndex].GetName(Database));
        
        writer.WriteStartArray("Properties");
        foreach (var prop in Database.GetProperties(structIndex))
        {
            writer.WriteStartObject();
            writer.WriteString("Name", prop.GetName(Database));
            writer.WriteString("Type", prop.GetTypeString(Database));
            writer.WriteEndObject();
        }
        writer.WriteEndArray();
        writer.WriteEndObject();
    }

    public void SaveEnumToFile(int enumIndex, string path)
    {
        using var fileStream = new FileStream(Path.ChangeExtension(path, "json"), FileMode.Create);
        using var writer = new Utf8JsonWriter(fileStream, new JsonWriterOptions
        {
            Indented = true,
        });
        
        var enumDefinition = Database.EnumDefinitions[enumIndex];
        
        writer.WriteStartObject();
        writer.WriteString("Name", enumDefinition.GetName(Database));
        
        writer.WriteStartArray("Values");
        
        for (var i = 0; i < enumDefinition.ValueCount; i++)
        {
            var enumOption = Database.EnumOptions[enumDefinition.FirstValueIndex + i].ToString(Database);
            writer.WriteStringValue(enumOption);
        }

        writer.WriteEndArray();
        writer.WriteEndObject();
    }

    public string GetFromMainRecord(DataCoreRecord record)
    {
        if (!Database.MainRecords.Contains(record.Id))
            throw new InvalidOperationException("Can only extract main records");

        using var stringWriter = new MemoryStream();
        using var writer = new Utf8JsonWriter(stringWriter, new JsonWriterOptions
        {
            Indented = false
        });

        WriteInner(record, writer);

        return Encoding.UTF8.GetString(stringWriter.ToArray());
    }

    private void WriteInner(DataCoreRecord record, Utf8JsonWriter writer)
    {
        var context = new Context(record.GetFileName(Database), writer, DataCoreBinaryWalker.WalkRecord(record, Database));

        context.Writer.WriteStartObject();
        context.Writer.WriteString("_RecordName_", record.GetName(Database));
        context.Writer.WriteString("_RecordId_", record.Id.ToString());
        var tag = record.GetTag(Database);
        if (tag is not null)
            context.Writer.WriteString("_RecordTag_", tag);
        context.Writer.WriteStartObject("_RecordValue_");
        WriteInstance(record.StructIndex, record.InstanceIndex, context);
        context.Writer.WriteEndObject();
        
        if (context.PointedTo.Count > 0)
        {
            Debugger.Break();
            Console.WriteLine($"Warning: {context.PointedTo.Count} pointers were not resolved");
            context.Writer.WriteStartObject("_Pointers_");

            foreach (var (structIndex, instanceIndex) in context.PointedTo)
            {
                context.Writer.WriteString($"ptr:{context.GetPointer(structIndex, instanceIndex)}", $"struct:{structIndex},instance:{instanceIndex}");
            }

            context.Writer.WriteEndObject();
        }

        context.Writer.WriteEndObject();

        context.Writer.Flush();
    }

    private void WriteInstance(int structIndex, int instanceIndex, Context context)
    {
        var reader = Database.GetReader(structIndex, instanceIndex);

        if (context.TryGetPointer(structIndex, instanceIndex, out var pointerIndex))
            context.Writer.WriteString("_Pointer_", $"ptr:{pointerIndex}");

        WriteStruct(structIndex, ref reader, context);
    }

    private void WriteStruct(int structIndex, ref SpanReader reader, Context context)
    {
        context.Writer.WriteString("_Type_", Database.StructDefinitions[structIndex].GetName(Database));

        foreach (var prop in Database.GetProperties(structIndex))
        {
            if (prop.ConversionType == ConversionType.Attribute)
                WriteAttribute(prop, ref reader, context);
            else
                WriteArray(prop, ref reader, context);
        }
    }

    private void WriteAttribute(DataCorePropertyDefinition prop, ref SpanReader reader, Context context)
    {
        var propName = prop.GetName(Database);

        switch (prop.DataType)
        {
            case DataType.Class:
                context.Writer.WriteStartObject(propName);
                WriteStruct(prop.StructIndex, ref reader, context);
                context.Writer.WriteEndObject();
                break;
            case DataType.WeakPointer: WriteFromWeakPointer(reader.Read<DataCorePointer>(), context, propName); break;
            case DataType.StrongPointer: WriteFromStrongPointer(reader.Read<DataCorePointer>(), context, propName); break;
            case DataType.Reference: WriteFromReference(reader.Read<DataCoreReference>(), context, propName); break;
            case DataType.EnumChoice: context.Writer.WriteString(propName, reader.Read<DataCoreStringId>().ToString(Database)); break;
            case DataType.Guid: context.Writer.WriteString(propName, reader.Read<CigGuid>().ToString()); break;
            case DataType.Locale: context.Writer.WriteString(propName, reader.Read<DataCoreStringId>().ToString(Database)); break;
            case DataType.Double: context.Writer.WriteNumber(propName, reader.ReadDouble()); break;
            case DataType.Single: context.Writer.WriteNumber(propName, reader.ReadSingle()); break;
            case DataType.String: context.Writer.WriteString(propName, reader.Read<DataCoreStringId>().ToString(Database)); break;
            case DataType.UInt64: context.Writer.WriteNumber(propName, reader.ReadUInt64()); break;
            case DataType.UInt32: context.Writer.WriteNumber(propName, reader.ReadUInt32()); break;
            case DataType.UInt16: context.Writer.WriteNumber(propName, reader.ReadUInt16()); break;
            case DataType.Byte: context.Writer.WriteNumber(propName, reader.ReadByte()); break;
            case DataType.Int64: context.Writer.WriteNumber(propName, reader.ReadInt64()); break;
            case DataType.Int32: context.Writer.WriteNumber(propName, reader.ReadInt32()); break;
            case DataType.Int16: context.Writer.WriteNumber(propName, reader.ReadInt16()); break;
            case DataType.SByte: context.Writer.WriteNumber(propName, reader.ReadSByte()); break;
            case DataType.Boolean: context.Writer.WriteBoolean(propName, reader.ReadBoolean()); break;
            default:
                throw new ArgumentOutOfRangeException();
        }
    }

    private void WriteArray(DataCorePropertyDefinition prop, ref SpanReader reader, Context context)
    {
        var count = reader.ReadInt32();
        var firstIndex = reader.ReadInt32();

        var propName = prop.GetName(Database);
        context.Writer.WriteStartArray(propName);

        for (var i = firstIndex; i < firstIndex + count; i++)
        {
            switch (prop.DataType)
            {
                case DataType.Class:
                    context.Writer.WriteStartObject();
                    WriteInstance(prop.StructIndex, i, context);
                    context.Writer.WriteEndObject();
                    break;
                case DataType.Reference: WriteFromReference(Database.ReferenceValues[i], context, null); break;
                case DataType.WeakPointer: WriteFromWeakPointer(Database.WeakValues[i], context, null); break;
                case DataType.StrongPointer: WriteFromStrongPointer(Database.StrongValues[i], context, null); break;
                case DataType.EnumChoice: context.Writer.WriteStringValue(Database.EnumValues[i].ToString(Database)); break;
                case DataType.Guid: context.Writer.WriteStringValue(Database.GuidValues[i].ToString()); break;
                case DataType.Locale: context.Writer.WriteStringValue(Database.LocaleValues[i].ToString(Database)); break;
                case DataType.Double: context.Writer.WriteNumberValue(Database.DoubleValues[i]); break;
                case DataType.Single: context.Writer.WriteNumberValue(Database.SingleValues[i]); break;
                case DataType.String: context.Writer.WriteStringValue(Database.StringIdValues[i].ToString(Database)); break;
                case DataType.UInt64: context.Writer.WriteNumberValue(Database.UInt64Values[i]); break;
                case DataType.UInt32: context.Writer.WriteNumberValue(Database.UInt32Values[i]); break;
                case DataType.UInt16: context.Writer.WriteNumberValue(Database.UInt16Values[i]); break;
                case DataType.Byte: context.Writer.WriteNumberValue(Database.UInt8Values[i]); break;
                case DataType.Int64: context.Writer.WriteNumberValue(Database.Int64Values[i]); break;
                case DataType.Int32: context.Writer.WriteNumberValue(Database.Int32Values[i]); break;
                case DataType.Int16: context.Writer.WriteNumberValue(Database.Int16Values[i]); break;
                case DataType.SByte: context.Writer.WriteNumberValue(Database.Int8Values[i]); break;
                case DataType.Boolean: context.Writer.WriteBooleanValue(Database.BooleanValues[i]); break;
                default: throw new ArgumentOutOfRangeException();
            }
        }

        context.Writer.WriteEndArray();
    }

    private void WriteFromReference(DataCoreReference reference, Context context, string? propName)
    {
        if (reference.RecordId == CigGuid.Empty || reference.InstanceIndex == -1)
        {
            if (propName == null)
                context.Writer.WriteNullValue();
            else
                context.Writer.WriteNull(propName);

            return;
        }

        var record = Database.GetRecord(reference.RecordId);

        if (Database.MainRecords.Contains(reference.RecordId))
        {
            //if we're referencing a full on file, just add a small mention to it
            var relativePath = Path.ChangeExtension(DataCoreUtils.ComputeRelativePath(record.GetFileName(Database), context.Path), "json");
            if (propName != null)
                context.Writer.WriteString(propName, relativePath);
            else
                context.Writer.WriteStringValue(relativePath);

            return;
        }

        if (record.GetFileName(Database) == context.Path)
        {
            //if we're referencing a record in the same file, we have to write it out
            if (propName == null)
                context.Writer.WriteStartObject();
            else
                context.Writer.WriteStartObject(propName);

            context.Writer.WriteString("_RecordId_", record.Id.ToString());
            context.Writer.WriteString("_RecordName_", record.GetName(Database));
            WriteInstance(record.StructIndex, record.InstanceIndex, context);
            context.Writer.WriteEndObject();

            return;
        }

        //if we get here, we're referencing a part of another file. mention the file and some details
        if (propName == null)
            context.Writer.WriteStartObject();
        else
            context.Writer.WriteStartObject(propName);

        context.Writer.WriteString("_RecordPath_", Path.ChangeExtension(DataCoreUtils.ComputeRelativePath(record.GetFileName(Database), context.Path), "json"));
        context.Writer.WriteString("_RecordName_", record.GetName(Database));
        context.Writer.WriteString("_RecordId_", record.Id.ToString());

        context.Writer.WriteEndObject();
    }

    private void WriteFromStrongPointer(DataCorePointer strongPointer, Context context, string? propName)
    {
        if (strongPointer.StructIndex == -1 || strongPointer.InstanceIndex == -1)
        {
            if (propName == null)
                context.Writer.WriteNullValue();
            else
                context.Writer.WriteNull(propName);

            return;
        }

        if (propName == null)
            context.Writer.WriteStartObject();
        else
            context.Writer.WriteStartObject(propName);

        WriteInstance(strongPointer.StructIndex, strongPointer.InstanceIndex, context);
        context.Writer.WriteEndObject();
    }

    private static void WriteFromWeakPointer(DataCorePointer weakPointer, Context context, string? propName)
    {
        if (weakPointer.StructIndex == -1 || weakPointer.InstanceIndex == -1)
        {
            if (propName == null)
                context.Writer.WriteNullValue();
            else
                context.Writer.WriteNull(propName);

            return;
        }

        var pointerIndex = context.GetPointer(weakPointer.StructIndex, weakPointer.InstanceIndex);
        var pointerValue = $"_PointsTo_:ptr:{pointerIndex}";

        if (propName == null)
            context.Writer.WriteStringValue(pointerValue);
        else
            context.Writer.WriteString(propName, pointerValue);
    }

    private readonly struct Context
    {
        private readonly Dictionary<(int, int), int> _pointers;
        public string Path { get; }
        public Utf8JsonWriter Writer { get; }
        public HashSet<(int, int)> PointedTo { get; }

        public Context(string path, Utf8JsonWriter writer, Dictionary<(int, int), int> pointers)
        {
            Path = path;
            Writer = writer;
            _pointers = pointers;
            PointedTo = _pointers.Keys.ToHashSet();
        }

        public int GetPointer(int structIndex, int instanceIndex) => _pointers[(structIndex, instanceIndex)];

        public bool TryGetPointer(int structIndex, int instanceIndex, out int pointerIndex)
        {
            PointedTo.Remove((structIndex, instanceIndex));

            return _pointers.TryGetValue((structIndex, instanceIndex), out pointerIndex);
        }
    }
}