using System.Globalization;
using System.Text;
using System.Xml;
using StarBreaker.Common;

namespace StarBreaker.DataCore;

public sealed class DataCoreBinaryXml : IDataCoreBinary<string>
{
    public DataCoreDatabase Database { get; }

    public DataCoreBinaryXml(DataCoreDatabase db)
    {
        Database = db;
    }

    public void SaveRecordToFile(DataCoreRecord record, string path)
    {
        using var fileStream = new FileStream(Path.ChangeExtension(path, "xml"), FileMode.Create);
        using var writer = XmlWriter.Create(fileStream, new XmlWriterSettings
        {
            Indent = true
        });

        WriteInner(record, writer);
    }

    public void SaveStructToFile(int structIndex, string path)
    {
        using var fileStream = new FileStream(Path.ChangeExtension(path, "xml"), FileMode.Create);
        using var writer = XmlWriter.Create(fileStream, new XmlWriterSettings
        {
            Indent = true
        });
        
        var structDefinition = Database.StructDefinitions[structIndex];
        
        writer.WriteStartElement("Struct");
        writer.WriteAttributeString("Name", structDefinition.GetName(Database));
        if (structDefinition.ParentTypeIndex != -1)
            writer.WriteAttributeString("Parent", Database.StructDefinitions[structDefinition.ParentTypeIndex].GetName(Database));

        foreach (var prop in Database.GetProperties(structIndex))
        {
            writer.WriteStartElement("Property");
            writer.WriteAttributeString("Name", prop.GetName(Database));
            writer.WriteAttributeString("Type", prop.GetTypeString(Database));
            writer.WriteEndElement();
        }
        writer.WriteEndElement();
    }
    
    public void SaveEnumToFile(int enumIndex, string path)
    {
        using var fileStream = new FileStream(Path.ChangeExtension(path, "xml"), FileMode.Create);
        using var writer = XmlWriter.Create(fileStream, new XmlWriterSettings
        {
            Indent = true
        });
        
        var enumDefinition = Database.EnumDefinitions[enumIndex];
        
        writer.WriteStartElement("Enum");
        writer.WriteAttributeString("Name", enumDefinition.GetName(Database));
        
        for (var i = 0; i < enumDefinition.ValueCount; i++)
        {
            var value = Database.EnumOptions[enumDefinition.FirstValueIndex + i];
            writer.WriteElementString("Value", value.ToString(Database));
        }
        writer.WriteEndElement();
    }

    public string GetFromMainRecord(DataCoreRecord record)
    {
        if (!Database.MainRecords.Contains(record.Id))
            throw new InvalidOperationException("Can only extract main records");

        using var ms = new MemoryStream();
        using var writer = XmlWriter.Create(ms, new XmlWriterSettings
        {
            Indent = true
        });

        WriteInner(record, writer);

        return Encoding.UTF8.GetString(ms.ToArray());
    }

    private void WriteInner(DataCoreRecord record, XmlWriter writer)
    {
        var context = new Context(record.GetFileName(Database), writer, DataCoreBinaryWalker.WalkRecord(record, Database));

        writer.WriteStartElement(XmlConvert.EncodeName(record.GetName(Database)));
        writer.WriteAttributeString("RecordId", record.Id.ToString());
        var tag = record.GetTag(Database);
        if (tag is not null)
            writer.WriteAttributeString("RecordTag", tag);

        WriteInstance(record.StructIndex, record.InstanceIndex, context);

        writer.WriteEndElement();

        writer.Flush();
    }

    private void WriteInstance(int structIndex, int instanceIndex, Context context)
    {
        var reader = Database.GetReader(structIndex, instanceIndex);

        if (context.Pointers.ContainsKey((structIndex, instanceIndex)))
            context.Writer.WriteAttributeString("Pointer", context.GetPointer(structIndex, instanceIndex));

        WriteStruct(structIndex, ref reader, context);
    }

    private void WriteStruct(int structIndex, ref SpanReader reader, Context context)
    {
        context.Writer.WriteAttributeString("Type", Database.StructDefinitions[structIndex].GetName(Database));

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
            case DataType.Reference:
                context.Writer.WriteStartElement(propName);
                WriteFromReference(reader.Read<DataCoreReference>(), context);
                context.Writer.WriteEndElement();
                break;
            case DataType.WeakPointer:
                var weakPointer = reader.Read<DataCorePointer>();
                context.Writer.WriteStartElement(propName);
                if (weakPointer.StructIndex != -1 && weakPointer.InstanceIndex != -1)
                    context.Writer.WriteAttributeString("PointsTo", context.GetPointer(weakPointer.StructIndex, weakPointer.InstanceIndex));
                context.Writer.WriteEndElement();
                break;
            case DataType.StrongPointer:
                var strongPointer = reader.Read<DataCorePointer>();
                context.Writer.WriteStartElement(propName);
                if (strongPointer.StructIndex != -1 && strongPointer.InstanceIndex != -1)
                    WriteInstance(strongPointer.StructIndex, strongPointer.InstanceIndex, context);
                context.Writer.WriteEndElement();
                break;
            case DataType.Class:
                context.Writer.WriteStartElement(propName);
                WriteStruct(prop.StructIndex, ref reader, context);
                context.Writer.WriteEndElement();
                break;
            case DataType.EnumChoice: context.Writer.WriteElementString(propName, reader.Read<DataCoreStringId>().ToString(Database)); break;
            case DataType.Guid: context.Writer.WriteElementString(propName, reader.Read<CigGuid>().ToString()); break;
            case DataType.Locale: context.Writer.WriteElementString(propName, reader.Read<DataCoreStringId>().ToString(Database)); break;
            case DataType.Double: context.Writer.WriteElementString(propName, reader.ReadDouble().ToString(CultureInfo.InvariantCulture)); break;
            case DataType.Single: context.Writer.WriteElementString(propName, reader.ReadSingle().ToString(CultureInfo.InvariantCulture)); break;
            case DataType.String: context.Writer.WriteElementString(propName, reader.Read<DataCoreStringId>().ToString(Database)); break;
            case DataType.UInt64: context.Writer.WriteElementString(propName, reader.ReadUInt64().ToString(CultureInfo.InvariantCulture)); break;
            case DataType.UInt32: context.Writer.WriteElementString(propName, reader.ReadUInt32().ToString(CultureInfo.InvariantCulture)); break;
            case DataType.UInt16: context.Writer.WriteElementString(propName, reader.ReadUInt16().ToString(CultureInfo.InvariantCulture)); break;
            case DataType.Byte: context.Writer.WriteElementString(propName, reader.ReadByte().ToString(CultureInfo.InvariantCulture)); break;
            case DataType.Int64: context.Writer.WriteElementString(propName, reader.ReadInt64().ToString(CultureInfo.InvariantCulture)); break;
            case DataType.Int32: context.Writer.WriteElementString(propName, reader.ReadInt32().ToString(CultureInfo.InvariantCulture)); break;
            case DataType.Int16: context.Writer.WriteElementString(propName, reader.ReadInt16().ToString(CultureInfo.InvariantCulture)); break;
            case DataType.SByte: context.Writer.WriteElementString(propName, reader.ReadSByte().ToString(CultureInfo.InvariantCulture)); break;
            case DataType.Boolean: context.Writer.WriteElementString(propName, reader.ReadBoolean().ToString(CultureInfo.InvariantCulture)); break;
            default:
                throw new ArgumentOutOfRangeException();
        }
    }

    private void WriteArray(DataCorePropertyDefinition prop, ref SpanReader reader, Context context)
    {
        var count = reader.ReadInt32();
        var firstIndex = reader.ReadInt32();

        var propName = prop.GetName(Database);
        context.Writer.WriteStartElement(propName);
        context.Writer.WriteAttributeString("Type", Database.StructDefinitions[prop.StructIndex].GetName(Database));
        context.Writer.WriteAttributeString("Count", count.ToString(CultureInfo.InvariantCulture));

        for (var i = firstIndex; i < firstIndex + count; i++)
        {
            switch (prop.DataType)
            {
                case DataType.Reference:
                    var reference = Database.ReferenceValues[i];

                    if (reference.RecordId == CigGuid.Empty || reference.InstanceIndex == -1)
                    {
                        // we don't know the type of this array element. Write the type of the array (which might be a base class)
                        context.Writer.WriteElementString(Database.StructDefinitions[prop.StructIndex].GetName(Database), null);
                    }
                    else
                    {
                        // we know the type of this array element. Write the actual type
                        var record = Database.GetRecord(reference.RecordId);
                        var actualTypeName = Database.StructDefinitions[record.StructIndex].GetName(Database);
                        context.Writer.WriteStartElement(actualTypeName);
                        WriteFromReference(Database.ReferenceValues[i], context);
                        context.Writer.WriteEndElement();
                    }

                    break;
                case DataType.WeakPointer:
                    var weakPointer = Database.WeakValues[i];
                    if (weakPointer.StructIndex == -1 || weakPointer.InstanceIndex == -1)
                    {
                        // we don't know the type of this array element. Write the type of the array (which might be a base class)
                        context.Writer.WriteElementString(Database.StructDefinitions[prop.StructIndex].GetName(Database), null);
                    }
                    else
                    {
                        context.Writer.WriteStartElement(Database.StructDefinitions[weakPointer.StructIndex].GetName(Database));
                        context.Writer.WriteAttributeString("PointsTo", context.GetPointer(weakPointer.StructIndex, weakPointer.InstanceIndex));
                        context.Writer.WriteEndElement();
                    }

                    break;
                case DataType.StrongPointer:
                    var strongPointer = Database.StrongValues[i];
                    if (strongPointer.StructIndex == -1 || strongPointer.InstanceIndex == -1)
                    {
                        // we don't know the type of this array element. Write the type of the array (which might be a base class)
                        context.Writer.WriteElementString(Database.StructDefinitions[prop.StructIndex].GetName(Database), null);
                    }
                    else
                    {
                        context.Writer.WriteStartElement(Database.StructDefinitions[strongPointer.StructIndex].GetName(Database));
                        WriteInstance(strongPointer.StructIndex, strongPointer.InstanceIndex, context);
                        context.Writer.WriteEndElement();
                    }

                    break;
                case DataType.Class:
                    context.Writer.WriteStartElement(Database.StructDefinitions[prop.StructIndex].GetName(Database));
                    WriteInstance(prop.StructIndex, i, context);
                    context.Writer.WriteEndElement();
                    break;
                case DataType.EnumChoice: context.Writer.WriteElementString(prop.DataType.ToStringFast(), Database.EnumValues[i].ToString(Database)); break;
                case DataType.Guid: context.Writer.WriteElementString(prop.DataType.ToStringFast(), Database.GuidValues[i].ToString()); break;
                case DataType.Locale: context.Writer.WriteElementString(prop.DataType.ToStringFast(), Database.LocaleValues[i].ToString(Database)); break;
                case DataType.Double: context.Writer.WriteElementString(prop.DataType.ToStringFast(), Database.DoubleValues[i].ToString(CultureInfo.InvariantCulture)); break;
                case DataType.Single: context.Writer.WriteElementString(prop.DataType.ToStringFast(), Database.SingleValues[i].ToString(CultureInfo.InvariantCulture)); break;
                case DataType.String: context.Writer.WriteElementString(prop.DataType.ToStringFast(), Database.StringIdValues[i].ToString(Database)); break;
                case DataType.UInt64: context.Writer.WriteElementString(prop.DataType.ToStringFast(), Database.UInt64Values[i].ToString(CultureInfo.InvariantCulture)); break;
                case DataType.UInt32: context.Writer.WriteElementString(prop.DataType.ToStringFast(), Database.UInt32Values[i].ToString(CultureInfo.InvariantCulture)); break;
                case DataType.UInt16: context.Writer.WriteElementString(prop.DataType.ToStringFast(), Database.UInt16Values[i].ToString(CultureInfo.InvariantCulture)); break;
                case DataType.Byte: context.Writer.WriteElementString(prop.DataType.ToStringFast(), Database.UInt8Values[i].ToString(CultureInfo.InvariantCulture)); break;
                case DataType.Int64: context.Writer.WriteElementString(prop.DataType.ToStringFast(), Database.Int64Values[i].ToString(CultureInfo.InvariantCulture)); break;
                case DataType.Int32: context.Writer.WriteElementString(prop.DataType.ToStringFast(), Database.Int32Values[i].ToString(CultureInfo.InvariantCulture)); break;
                case DataType.Int16: context.Writer.WriteElementString(prop.DataType.ToStringFast(), Database.Int16Values[i].ToString(CultureInfo.InvariantCulture)); break;
                case DataType.SByte: context.Writer.WriteElementString(prop.DataType.ToStringFast(), Database.Int8Values[i].ToString(CultureInfo.InvariantCulture)); break;
                case DataType.Boolean: context.Writer.WriteElementString(prop.DataType.ToStringFast(), Database.BooleanValues[i].ToString(CultureInfo.InvariantCulture)); break;
                default: throw new ArgumentOutOfRangeException();
            }
        }

        context.Writer.WriteEndElement();
    }

    private void WriteFromReference(DataCoreReference reference, Context context)
    {
        if (reference.RecordId == CigGuid.Empty || reference.InstanceIndex == -1)
            return;

        var record = Database.GetRecord(reference.RecordId);

        if (Database.MainRecords.Contains(reference.RecordId))
        {
            //if we're referencing a full on file, just add a small mention to it
            context.Writer.WriteAttributeString("ReferencedFile", DataCoreUtils.ComputeRelativePath(record.GetFileName(Database), context.Path));
            return;
        }

        if (record.GetFileName(Database) == context.Path)
        {
            //if we're referencing a record in the same file, write it out.
            //This should ensure that when exporting the entire datacore, each record is only written once.
            context.Writer.WriteAttributeString("RecordId", reference.RecordId.ToString());
            context.Writer.WriteAttributeString("RecordName", record.GetName(Database));
            WriteInstance(record.StructIndex, record.InstanceIndex, context);
            return;
        }
        
        //if we're referencing a record that's part of another file, mention it
        context.Writer.WriteAttributeString("RecordReference", DataCoreUtils.ComputeRelativePath(record.GetFileName(Database), context.Path));
        context.Writer.WriteAttributeString("RecordName", record.GetName(Database));
        context.Writer.WriteAttributeString("RecordId", reference.RecordId.ToString());
    }

    private readonly struct Context
    {
        public string Path { get; }
        public XmlWriter Writer { get; }
        public Dictionary<(int, int), int> Pointers { get; }

        public string GetPointer(int structIndex, int instanceIndex) => $"ptr:{Pointers[(structIndex, instanceIndex)]}";

        public Context(string path, XmlWriter writer, Dictionary<(int, int), int> pointers)
        {
            Path = path;
            Writer = writer;
            Pointers = pointers;
        }
    }
}